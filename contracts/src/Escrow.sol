// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {SignatureCheckerLib} from "solady/utils/SignatureCheckerLib.sol";
import {FeeMath} from "./libraries/FeeMath.sol";
import {EscrowConfig} from "./EscrowConfig.sol";
import {MarketBase} from "./MarketBase.sol";

/// @title Escrow — one-time escrow + the shared seller bond pool (plan-v3 §6)
/// @notice Funds can only flow to the order's buyer, seller, and the fee recipient (fees, base gas, dispute fees).
///         No owner, no admin withdrawal, no proxy, no selfdestruct. All owner-controlled settings live in
///         `EscrowConfig` and are snapshotted into each order at creation.
/// @dev Settlement model: every ending is expressed as `s` = the seller's share of the order principal (`amount`).
///      The fee pool (fees + base gas) is kept apart from principal; fees are earned pro rata to `s`, base gas is
///      earned in full once the platform has sponsored any post-payment operation (`platformPaid`).
contract Escrow is MarketBase, EIP712 {
    using SafeERC20 for IERC20;

    enum State {
        None,
        Funded,
        Disputed,
        Released,
        Refunded,
        Settled
    }

    struct Offer {
        address seller;
        address buyer; // 0 = public offer
        address token;
        uint256 amount;
        FeeMath.Payer feePayer;
        uint16 maxFeeBps;
        uint256 maxBaseGas;
        uint256 maxMinFee; // highest minimum fee the seller accepts (0 applies during a campaign)
        uint256 bond; // seller bond locked with the order (0 = none, ≤ amount)
        uint16 lateSlashBps; // share of the bond paid to the buyer if the seller misses the ship deadline
        bool instant;
        uint32 shipWindow;
        uint32 confirmWindow;
        bytes32 listingHash;
        uint64 expiry;
        uint32 maxFills;
        uint256 nonce;
    }

    /// @notice Seller's one-time authorization for an off-chain auction (bids never touch the chain).
    struct AuctionAuth {
        address seller;
        address token;
        uint256 auctionId;
        uint256 reservePrice;
        FeeMath.Payer feePayer;
        uint16 maxFeeBps;
        uint256 maxBaseGas;
        uint256 maxMinFee;
        bool instant;
        uint32 shipWindow;
        uint32 confirmWindow;
        bytes32 listingHash;
        uint64 expiry;
    }

    /// @notice Platform's proof of who won and at what price. Bound to the exact seller authorization by `authHash`.
    struct SettlementProof {
        bytes32 authHash;
        address winner;
        uint256 price;
        uint64 payDeadline;
    }

    struct Order {
        address buyer;
        uint64 shipDeadline;
        uint32 confirmWindow;
        address seller;
        uint64 confirmDeadline; // 0 = not delivered
        State state;
        bool platformPaid;
        bool buyerExtended;
        bool sellerDisputed;
        address token;
        uint64 disputedAt;
        uint32 resolveTimeout;
        uint128 amount;
        uint128 disputeFee;
        uint128 buyerFee;
        uint128 sellerFee;
        uint128 buyerGas;
        uint128 sellerGas;
        address proposer;
        uint64 proposalExpiry;
        uint32 proposalWindow;
        uint128 proposalToBuyer;
        uint128 bond;
        uint32 sellerExtended;
        uint32 arbExtended;
        uint16 lateSlashBps;
        bool disputeFromBond;
    }

    struct Bond {
        uint128 total;
        uint128 locked;
    }

    bytes32 public constant OFFER_TYPEHASH = keccak256(
        "Offer(address seller,address buyer,address token,uint256 amount,uint8 feePayer,uint16 maxFeeBps,uint256 maxBaseGas,uint256 maxMinFee,uint256 bond,uint16 lateSlashBps,bool instant,uint32 shipWindow,uint32 confirmWindow,bytes32 listingHash,uint64 expiry,uint32 maxFills,uint256 nonce)"
    );
    bytes32 public constant AUCTION_TYPEHASH = keccak256(
        "AuctionAuth(address seller,address token,uint256 auctionId,uint256 reservePrice,uint8 feePayer,uint16 maxFeeBps,uint256 maxBaseGas,uint256 maxMinFee,bool instant,uint32 shipWindow,uint32 confirmWindow,bytes32 listingHash,uint64 expiry)"
    );
    bytes32 public constant PROOF_TYPEHASH =
        keccak256("SettlementProof(bytes32 authHash,address winner,uint256 price,uint64 payDeadline)");

    uint256 internal constant BPS = 10_000;

    uint256 public constant MIN_WINDOW = 1 days;
    uint256 public constant MAX_WINDOW = 30 days;
    uint256 public constant MAX_SELLER_EXTENSION = 30 days;
    uint256 public constant EMERGENCY_WINDOW = 6 hours;
    uint256 public constant EMERGENCY_EXTENSION = 24 hours;
    uint256 public constant MAX_ARBITER_EXTENSION_CALL = 7 days;
    uint256 public constant MAX_ARBITER_EXTENSION = 14 days;

    EscrowConfig public immutable config;
    address public immutable arbiter;
    address public immutable feeRecipient;

    uint256 public orderCount;
    // slither-disable-next-line uninitialized-state (written through a storage pointer in `_record`)
    mapping(uint256 => Order) internal _orders;
    mapping(address => mapping(address => Bond)) internal _bonds; // seller => token => bond
    mapping(address => mapping(address => mapping(address => uint256))) public lockedBy; // market => seller => token
    mapping(bytes32 => bool) public auctionSettled; // keccak(seller, auctionId)

    event OrderCreated(
        uint256 indexed orderId,
        bytes32 indexed offerHash,
        address indexed buyer,
        address seller,
        address token,
        uint256 amount,
        bytes32 listingHash,
        bool instant
    );
    event AuctionSettled(uint256 indexed orderId, address indexed seller, uint256 indexed auctionId);
    event Delivered(uint256 indexed orderId, uint64 confirmDeadline);
    event ConfirmExtended(uint256 indexed orderId, address indexed by, uint64 confirmDeadline);
    event DeadlineExtended(uint256 indexed orderId, uint64 deadline);
    event EndProposed(uint256 indexed orderId, address indexed proposer, uint256 toBuyer, uint64 expiry);
    event Disputed(uint256 indexed orderId, address indexed by, uint256 disputeFee, bool fromBond);
    event Resolved(uint256 indexed orderId, uint256 toBuyerBps, uint256 bondToBuyerBps);
    event Settled(uint256 indexed orderId, State state, uint256 toBuyer, uint256 toSeller, uint256 toFeeRecipient);
    event BondDeposited(address indexed seller, address indexed token, uint256 amount);
    event BondWithdrawn(address indexed seller, address indexed token, address to, uint256 amount);
    event BondLocked(address indexed market, address indexed seller, address indexed token, uint256 amount);
    event BondTaken(address indexed market, address indexed seller, address indexed token, uint256 amount);
    event BondReleased(
        address indexed market, address indexed seller, address indexed token, uint256 amount, uint256 toBuyer
    );

    error BadOffer();
    error FeeLimit();
    error FeesExceedAmount();
    error BadState();
    error NotParty();
    error TooEarly();
    error TooLate();
    error BadParams();
    error InsufficientBond();

    constructor(EscrowConfig config_, address arbiter_, address feeRecipient_) EIP712("AkariEscrow", "1") {
        if (address(config_) == address(0) || arbiter_ == address(0) || feeRecipient_ == address(0)) {
            revert BadParams();
        }
        config = config_;
        arbiter = arbiter_;
        feeRecipient = feeRecipient_;
    }

    // ───────────────────────────── Orders ─────────────────────────────

    /// @notice Buyer verifies the seller's signed offer, opens the order and pays in one call.
    function createAndFund(Offer calldata offer, bytes calldata sellerSig)
        external
        nonReentrant
        returns (uint256 orderId)
    {
        if (offer.buyer != address(0) && offer.buyer != msg.sender) revert BadOffer();
        bytes32 offerHash = hashOffer(offer);
        _useOffer(offer.seller, offer.nonce, offer.expiry, offer.maxFills, offerHash, sellerSig);
        orderId = _createOrder(offer, offerHash, config.SCOPE_ESCROW());
    }

    /// @notice Auction winner pays and opens the order: the seller's authorization and the platform's settlement
    ///         proof must both verify, the price must reach the reserve, and each auction settles at most once.
    function createFromAuction(
        AuctionAuth calldata auth,
        bytes calldata sellerSig,
        SettlementProof calldata proof,
        bytes calldata platformSig
    ) external nonReentrant returns (uint256 orderId) {
        bytes32 authHash = hashAuction(auth);
        if (proof.authHash != authHash || msg.sender != proof.winner) revert BadOffer();
        if (block.timestamp > proof.payDeadline || block.timestamp > auth.expiry) revert OfferUnavailable();
        if (proof.price < auth.reservePrice) revert BadOffer();
        bytes32 key = keccak256(abi.encode(auth.seller, auth.auctionId));
        if (auctionSettled[key]) revert OfferUnavailable();
        auctionSettled[key] = true;
        {
            address signer = config.auctionSigner();
            if (signer == address(0) || !SignatureCheckerLib.isValidSignatureNow(signer, hashProof(proof), platformSig))
            {
                revert BadSignature();
            }
        }
        if (!SignatureCheckerLib.isValidERC6492SignatureNow(auth.seller, authHash, sellerSig)) revert BadSignature();
        orderId = _createOrder(_auctionTerms(auth, proof.price), authHash, config.SCOPE_AUCTION());
        emit AuctionSettled(orderId, auth.seller, auth.auctionId);
    }

    /// @notice Seller marks the goods as shipped; the buyer's confirmation window starts.
    function markDelivered(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (msg.sender != o.seller) revert NotParty();
        if (o.confirmDeadline != 0) revert BadState();
        if (block.timestamp > o.shipDeadline) revert TooLate();
        o.platformPaid = true;
        o.confirmDeadline = uint64(block.timestamp + o.confirmWindow);
        emit Delivered(orderId, o.confirmDeadline);
    }

    /// @notice Buyer confirms receipt; full release to the seller.
    function release(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (msg.sender != o.buyer) revert NotParty();
        o.platformPaid = true;
        _settle(orderId, State.Released, o.amount, 0, 0);
    }

    /// @notice Seller voluntarily refunds in full. No fee is charged and the bond is not slashed.
    function refund(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (msg.sender != o.seller) revert NotParty();
        _settle(orderId, State.Refunded, 0, 0, 0);
    }

    /// @notice Anyone: release after the confirm window, or refund when the seller missed the ship deadline
    ///         (then `lateSlashBps` of the bond goes to the buyer).
    function claimTimeout(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (o.confirmDeadline != 0) {
            if (block.timestamp <= o.confirmDeadline) revert TooEarly();
            o.platformPaid = true; // the worker submits this release on the platform's gas
            _settle(orderId, State.Released, o.amount, 0, 0);
        } else {
            if (block.timestamp <= o.shipDeadline) revert TooEarly();
            _settle(orderId, State.Refunded, 0, 0, o.lateSlashBps);
        }
    }

    /// @notice Seller extends the buyer's confirm window (cumulative cap), or the buyer takes a one-time 24h
    ///         emergency extension inside the last 6 hours (time to top up the dispute fee from an exchange).
    function extendConfirm(uint256 orderId, uint32 secs) external nonReentrant {
        Order storage o = _funded(orderId);
        uint256 deadline = o.confirmDeadline;
        if (deadline == 0) revert BadState();
        if (block.timestamp > deadline) revert TooLate();
        if (msg.sender == o.seller) {
            uint256 total = uint256(o.sellerExtended) + secs;
            if (secs == 0 || total > MAX_SELLER_EXTENSION) revert BadParams();
            o.sellerExtended = uint32(total);
            deadline += secs;
        } else if (msg.sender == o.buyer) {
            if (o.buyerExtended) revert BadState();
            if (block.timestamp + EMERGENCY_WINDOW < deadline) revert TooEarly();
            o.buyerExtended = true;
            deadline += EMERGENCY_EXTENSION;
        } else {
            revert NotParty();
        }
        o.platformPaid = true;
        o.confirmDeadline = uint64(deadline);
        emit ConfirmExtended(orderId, msg.sender, uint64(deadline));
    }

    /// @notice Arbiter Safe, when the main site is down: extend open orders' ship / confirm deadlines. Never moves
    ///         funds. ≤ 7 days per call and ≤ 14 days per order in total; orders not open or over the cap are skipped.
    function extendDeadlines(uint256[] calldata orderIds, uint32 secs) external nonReentrant {
        if (msg.sender != arbiter) revert NotParty();
        if (secs == 0 || secs > MAX_ARBITER_EXTENSION_CALL) revert BadParams();
        for (uint256 i; i < orderIds.length; ++i) {
            Order storage o = _orders[orderIds[i]];
            uint256 total = uint256(o.arbExtended) + secs;
            if (o.state != State.Funded || total > MAX_ARBITER_EXTENSION) continue;
            o.arbExtended = uint32(total);
            uint64 deadline;
            if (o.confirmDeadline != 0) deadline = o.confirmDeadline += secs;
            else deadline = o.shipDeadline += secs;
            emit DeadlineExtended(orderIds[i], deadline);
        }
    }

    /// @notice Either party proposes how much principal goes back to the buyer; replaces any earlier proposal.
    function proposeEnd(uint256 orderId, uint256 toBuyer) external nonReentrant {
        Order storage o = _open(orderId);
        if (msg.sender != o.buyer && msg.sender != o.seller) revert NotParty();
        if (toBuyer > o.amount) revert BadParams();
        o.platformPaid = true;
        o.proposer = msg.sender;
        o.proposalToBuyer = uint128(toBuyer);
        o.proposalExpiry = uint64(block.timestamp + o.proposalWindow);
        emit EndProposed(orderId, msg.sender, toBuyer, o.proposalExpiry);
    }

    /// @notice The counterparty accepts the live proposal. `toBuyer` must match (guards against a swapped proposal).
    ///         Accepting during a dispute refunds the dispute fee in full.
    function acceptEnd(uint256 orderId, uint256 toBuyer) external nonReentrant {
        Order storage o = _open(orderId);
        if (msg.sender != o.buyer && msg.sender != o.seller) revert NotParty();
        if (o.proposer == address(0) || o.proposer == msg.sender) revert BadState();
        if (block.timestamp > o.proposalExpiry) revert TooLate();
        if (toBuyer != o.proposalToBuyer) revert BadParams();
        o.platformPaid = true;
        _settle(orderId, State.Settled, o.amount - toBuyer, 0, 0);
    }

    /// @notice Either party opens a dispute before the active deadline and prepays the dispute fee from the wallet.
    function dispute(uint256 orderId) external nonReentrant {
        _dispute(orderId, false);
    }

    /// @notice Seller opens a dispute paying the dispute fee from their available bond (no exchange top-up needed).
    function disputeWithBond(uint256 orderId) external nonReentrant {
        _dispute(orderId, true);
    }

    /// @notice Arbiter Safe executes a published ruling: `toBuyerBps` of the principal to the buyer, the rest to the
    ///         seller, and `bondToBuyerBps` of the locked bond to the buyer (the rest is unlocked for the seller).
    function resolve(uint256 orderId, uint256 toBuyerBps, uint256 bondToBuyerBps) external nonReentrant {
        if (msg.sender != arbiter) revert NotParty();
        Order storage o = _disputed(orderId);
        if (toBuyerBps > BPS || bondToBuyerBps > BPS) revert BadParams();
        uint256 disputerWinBps = o.sellerDisputed ? BPS - toBuyerBps : toBuyerBps;
        uint256 retained = uint256(o.disputeFee) * (BPS - disputerWinBps) / BPS;
        emit Resolved(orderId, toBuyerBps, bondToBuyerBps);
        _settle(orderId, State.Settled, uint256(o.amount) * (BPS - toBuyerBps) / BPS, retained, bondToBuyerBps);
    }

    /// @notice Anyone, once a dispute has gone unresolved for `resolveTimeout`: not delivered → full refund;
    ///         delivered → principal split 50/50. Dispute fee refunded in full; bond unlocked.
    function resolveTimeout(uint256 orderId) external nonReentrant {
        Order storage o = _disputed(orderId);
        if (block.timestamp <= uint256(o.disputedAt) + o.resolveTimeout) revert TooEarly();
        _settle(orderId, State.Settled, o.confirmDeadline != 0 ? o.amount / 2 : 0, 0, 0);
    }

    // ───────────────────────────── Seller bond pool ─────────────────────────────

    /// @notice One pool per seller and token, shared by every market (Escrow and StreamEscrow).
    function depositBond(address token, uint256 amount) external nonReentrant {
        if (amount == 0 || !config.tokenAllowed(token)) revert BadParams();
        _bonds[msg.sender][token].total += uint128(amount);
        config.lockFunds(amount, true);
        emit BondDeposited(msg.sender, token, amount);
        _pullExact(token, amount);
    }

    /// @notice Withdraw only the part of the bond not locked by any open order.
    function withdrawBond(address token, uint256 amount, address to) external nonReentrant {
        Bond storage b = _bonds[msg.sender][token];
        if (amount == 0 || amount > b.total - b.locked) revert InsufficientBond();
        b.total -= uint128(amount);
        config.unlockFunds(amount);
        emit BondWithdrawn(msg.sender, token, to, amount);
        IERC20(token).safeTransfer(to, amount);
    }

    /// @notice Another market (StreamEscrow) locks part of a seller's bond for one of its orders.
    function lockBond(address seller, address token, uint256 amount) external nonReentrant {
        if (!config.isMarket(msg.sender)) revert NotParty();
        _lockBond(seller, token, amount);
        lockedBy[msg.sender][seller][token] += amount;
        emit BondLocked(msg.sender, seller, token, amount);
    }

    /// @notice A market releases bond it locked, paying `toBuyer` of it to the buyer. It can never release more
    ///         than it locked (underflow reverts), and the slashed part can only go to that order's buyer.
    function releaseBond(address seller, address token, uint256 amount, address buyer, uint256 toBuyer)
        external
        nonReentrant
    {
        if (!config.isMarket(msg.sender)) revert NotParty();
        if (toBuyer > amount) revert BadParams();
        lockedBy[msg.sender][seller][token] -= amount;
        Bond storage b = _bonds[seller][token];
        b.locked -= uint128(amount);
        b.total -= uint128(toBuyer);
        emit BondReleased(msg.sender, seller, token, amount, toBuyer);
        if (toBuyer > 0) {
            config.unlockFunds(toBuyer);
            _push(token, buyer, toBuyer);
        }
    }

    /// @notice A market moves part of a seller's free (unlocked) bond to itself: the seller's own dispute fee for
    ///         one of that market's orders (StreamEscrow `disputeWithBond`). Total custody is unchanged.
    function takeBond(address seller, address token, uint256 amount) external nonReentrant {
        if (!config.isMarket(msg.sender)) revert NotParty();
        Bond storage b = _bonds[seller][token];
        if (amount > b.total - b.locked) revert InsufficientBond();
        b.total -= uint128(amount);
        emit BondTaken(msg.sender, seller, token, amount);
        IERC20(token).safeTransfer(msg.sender, amount);
    }

    // ───────────────────────────── Views ─────────────────────────────

    function getOrder(uint256 orderId) external view returns (Order memory) {
        return _orders[orderId];
    }

    function bondOf(address seller, address token)
        external
        view
        returns (uint256 total, uint256 locked, uint256 available)
    {
        Bond memory b = _bonds[seller][token];
        return (b.total, b.locked, b.total - b.locked);
    }

    /// @dev All three structs contain only static types, so `abi.encode(typehash, s)` is exactly EIP-712 `encodeData`.
    function hashOffer(Offer calldata o) public view returns (bytes32) {
        return _hashTypedDataV4(keccak256(abi.encode(OFFER_TYPEHASH, o)));
    }

    function hashAuction(AuctionAuth calldata a) public view returns (bytes32) {
        return _hashTypedDataV4(keccak256(abi.encode(AUCTION_TYPEHASH, a)));
    }

    function hashProof(SettlementProof calldata p) public view returns (bytes32) {
        return _hashTypedDataV4(keccak256(abi.encode(PROOF_TYPEHASH, p)));
    }

    // ───────────────────────────── Internal ─────────────────────────────

    /// @dev Shared tail of createAndFund / createFromAuction: snapshot params, check terms, record, lock, pull.
    function _createOrder(Offer memory o, bytes32 ref, uint8 scope) internal returns (uint256 orderId) {
        EscrowConfig.Quote memory q = config.checkout(o.token, scope);
        _checkTerms(o, q);
        orderId = ++orderCount;
        uint256 buyerPays = _record(orderId, o, q);
        if (o.bond > 0) _lockBond(o.seller, o.token, o.bond);
        config.lockFunds(buyerPays, true);
        emit OrderCreated(orderId, ref, msg.sender, o.seller, o.token, o.amount, o.listingHash, o.instant);
        if (o.instant) emit Delivered(orderId, _orders[orderId].confirmDeadline);
        _pullExact(o.token, buyerPays);
    }

    function _auctionTerms(AuctionAuth calldata a, uint256 price) internal view returns (Offer memory) {
        return Offer({
            seller: a.seller,
            buyer: msg.sender,
            token: a.token,
            amount: price,
            feePayer: a.feePayer,
            maxFeeBps: a.maxFeeBps,
            maxBaseGas: a.maxBaseGas,
            maxMinFee: a.maxMinFee,
            bond: 0,
            lateSlashBps: 0,
            instant: a.instant,
            shipWindow: a.shipWindow,
            confirmWindow: a.confirmWindow,
            listingHash: a.listingHash,
            expiry: a.expiry,
            maxFills: 1,
            nonce: 0
        });
    }

    /// @dev Writes the new order with its parameter snapshot; returns what the buyer must pay.
    function _record(uint256 orderId, Offer memory offer, EscrowConfig.Quote memory q) internal returns (uint256) {
        uint256 feeTotal = FeeMath.feeTotal(offer.amount, q.feeBps, q.minFee, q.campaign);
        (uint256 buyerFee, uint256 sellerFee) = FeeMath.split(feeTotal, offer.feePayer);
        (uint256 buyerGas, uint256 sellerGas) = FeeMath.split(q.baseGas, offer.feePayer);
        if (sellerFee + sellerGas >= offer.amount) revert FeesExceedAmount();

        Order storage o = _orders[orderId];
        o.buyer = msg.sender;
        o.seller = offer.seller;
        o.token = offer.token;
        o.state = State.Funded;
        o.amount = uint128(offer.amount);
        o.buyerFee = uint128(buyerFee);
        o.sellerFee = uint128(sellerFee);
        o.buyerGas = uint128(buyerGas);
        o.sellerGas = uint128(sellerGas);
        o.disputeFee = uint128(FeeMath.disputeFee(feeTotal, q.disputeMinFee, q.disputeMulBps));
        o.bond = uint128(offer.bond);
        o.lateSlashBps = offer.lateSlashBps;
        o.confirmWindow = offer.confirmWindow;
        o.resolveTimeout = q.resolveTimeout;
        o.proposalWindow = q.proposalWindow;
        if (offer.instant) {
            o.confirmDeadline = uint64(block.timestamp + offer.confirmWindow);
        } else {
            o.shipDeadline = uint64(block.timestamp + offer.shipWindow);
        }
        return FeeMath.buyerPays(offer.amount, buyerFee, buyerGas);
    }

    function _dispute(uint256 orderId, bool fromBond) internal {
        Order storage o = _funded(orderId);
        bool bySeller = msg.sender == o.seller;
        if (!bySeller && msg.sender != o.buyer) revert NotParty();
        if (fromBond && !bySeller) revert NotParty();
        uint256 deadline = o.confirmDeadline != 0 ? o.confirmDeadline : o.shipDeadline;
        if (block.timestamp > deadline) revert TooLate();
        o.state = State.Disputed;
        o.sellerDisputed = bySeller;
        o.disputedAt = uint64(block.timestamp);
        uint256 fee = o.disputeFee;
        emit Disputed(orderId, msg.sender, fee, fromBond);
        if (fromBond) {
            // Moves from the seller's free bond into the order: total custody is unchanged.
            Bond storage b = _bonds[o.seller][o.token];
            if (fee > b.total - b.locked) revert InsufficientBond();
            b.total -= uint128(fee);
            o.disputeFromBond = true;
        } else {
            config.lockFunds(fee, false); // never block a dispute on the global cap
            _pullExact(o.token, fee);
        }
    }

    function _settle(uint256 orderId, State end, uint256 s, uint256 disputeRetained, uint256 bondSlashBps) internal {
        Order storage o = _orders[orderId];
        (uint256 toBuyer, uint256 toSeller, uint256 toFeeRecipient) = _shares(o, s, disputeRetained);
        if (o.disputedAt != 0) {
            uint256 disputeRefund = o.disputeFee - disputeRetained;
            if (!o.sellerDisputed) toBuyer += disputeRefund;
            else if (o.disputeFromBond) _bonds[o.seller][o.token].total += uint128(disputeRefund); // stays in custody
            else toSeller += disputeRefund;
        }
        if (o.bond > 0) {
            Bond storage b = _bonds[o.seller][o.token];
            uint256 slash = uint256(o.bond) * bondSlashBps / BPS;
            b.locked -= o.bond;
            b.total -= uint128(slash);
            toBuyer += slash;
        }

        o.state = end;
        config.unlockFunds(toBuyer + toSeller + toFeeRecipient);
        emit Settled(orderId, end, toBuyer, toSeller, toFeeRecipient);
        address token = o.token;
        _push(token, o.buyer, toBuyer);
        _push(token, o.seller, toSeller);
        _push(token, feeRecipient, toFeeRecipient);
    }

    /// @dev Splits the order's principal + fee pool for seller share `s`. A retained dispute fee goes to the fee
    ///      recipient and replaces the platform fee (at most one of the two is charged).
    function _shares(Order storage o, uint256 s, uint256 disputeRetained)
        internal
        view
        returns (uint256 toBuyer, uint256 toSeller, uint256 toFeeRecipient)
    {
        (uint256 feeB, uint256 feeS) = FeeMath.feesCharged(o.buyerFee, o.sellerFee, s, o.amount, disputeRetained);
        bool gasEarned = o.platformPaid;
        uint256 gas = gasEarned ? uint256(o.buyerGas) + o.sellerGas : 0;
        uint256 sellerCost = feeS + (gasEarned ? o.sellerGas : 0);
        toSeller = s > sellerCost ? s - sellerCost : 0;
        toFeeRecipient = gas + feeB + feeS + disputeRetained;
        toBuyer = uint256(o.amount) + o.buyerFee + o.buyerGas + disputeRetained - toFeeRecipient - toSeller;
    }

    function _lockBond(address seller, address token, uint256 amount) internal {
        Bond storage b = _bonds[seller][token];
        if (amount > b.total - b.locked) revert InsufficientBond();
        b.locked += uint128(amount);
    }

    function _funded(uint256 orderId) internal view returns (Order storage o) {
        o = _orders[orderId];
        if (o.state != State.Funded) revert BadState();
    }

    function _disputed(uint256 orderId) internal view returns (Order storage o) {
        o = _orders[orderId];
        if (o.state != State.Disputed) revert BadState();
    }

    function _open(uint256 orderId) internal view returns (Order storage o) {
        o = _orders[orderId];
        if (o.state != State.Funded && o.state != State.Disputed) revert BadState();
    }

    function _checkTerms(Offer memory o, EscrowConfig.Quote memory q) internal view {
        if (o.seller == address(0) || o.seller == msg.sender) revert BadOffer();
        if (o.amount < q.minOrder || o.amount > q.orderCap) revert BadOffer();
        if (o.bond > o.amount || o.lateSlashBps > BPS) revert BadOffer();
        if (o.confirmWindow < MIN_WINDOW || o.confirmWindow > MAX_WINDOW) revert BadOffer();
        if (!o.instant && (o.shipWindow < MIN_WINDOW || o.shipWindow > MAX_WINDOW)) revert BadOffer();
        if (q.feeBps > o.maxFeeBps || q.baseGas > o.maxBaseGas || q.minFee > o.maxMinFee) revert FeeLimit();
    }
}
