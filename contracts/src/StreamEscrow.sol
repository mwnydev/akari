// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {FeeMath} from "./libraries/FeeMath.sol";
import {EscrowConfig} from "./EscrowConfig.sol";
import {Escrow} from "./Escrow.sol";
import {MarketBase} from "./MarketBase.sol";

/// @title StreamEscrow — installment escrow for time-based services (plan-v3 §6)
/// @notice The buyer pays in full up front; once the seller starts, the unlockable net principal unlocks period by
///         period and the seller claims it as it goes. Funds can only flow to the order's buyer, seller and the fee
///         recipient. No owner, no admin withdrawal, no proxy, no selfdestruct. Settings come from `EscrowConfig`
///         (snapshotted per order); seller bonds live in `Escrow`'s shared pool, which this market locks and releases.
/// @dev Settlement model: every ending is expressed as `s` = the seller's total share of the net principal `net`
///      (already-claimed amounts included). The fee pool (all fees + base gas, kept apart from `net` at creation) is
///      earned pro rata to `s / net`; base gas is earned in full once the platform has sponsored any post-payment
///      operation (`platformPaid`). Everything else goes back to the buyer.
contract StreamEscrow is MarketBase, EIP712 {
    enum State {
        None,
        Funded,
        Disputed,
        Released,
        Refunded,
        Settled
    }

    enum Interval {
        Hour,
        Day,
        Week,
        Month // 30 days
    }

    /// @notice Where a dispute fee was paid from (the seller can skip the exchange top-up).
    enum FeeSource {
        Wallet,
        Unlocked, // the seller's unlocked, unclaimed principal of this order
        Bond // the seller's available bond in Escrow
    }

    struct Offer {
        address seller;
        address buyer; // 0 = public offer
        address token;
        uint256 amount;
        FeeMath.Payer feePayer;
        uint16 maxFeeBps;
        uint256 maxBaseGas;
        uint256 maxClaimGas;
        uint256 maxMinFee; // highest minimum fee the seller accepts (0 applies during a campaign)
        uint256 bond; // seller bond locked with the order (0 = none, ≤ amount)
        uint16 lateSlashBps; // share of the bond paid to the buyer if the seller misses the start deadline
        Interval interval;
        uint32 periods;
        uint32 startWindow;
        bytes32 listingHash;
        uint64 expiry;
        uint32 maxFills;
        uint256 nonce;
    }

    struct Order {
        address buyer;
        uint64 startDeadline;
        uint32 interval; // seconds per period
        address seller;
        uint64 startedAt; // 0 = not started
        State state;
        bool platformPaid;
        bool sellerDisputed;
        FeeSource disputeSource;
        address token;
        uint64 disputedAt;
        uint32 periods;
        address proposer;
        uint64 proposalExpiry;
        uint32 proposalWindow;
        uint64 lastClaimAt;
        uint32 resolveTimeout;
        uint32 arbExtended;
        uint16 lateSlashBps;
        uint128 net; // unlockable net principal = amount - sellerFee - sellerGas
        uint128 claimed; // net principal already paid out by `claim` (claim gas included)
        uint128 buyerFee;
        uint128 sellerFee;
        uint128 buyerGas;
        uint128 sellerGas;
        uint128 disputeFee;
        uint128 claimGas;
        uint128 bond;
        uint128 proposalToBuyer;
    }

    bytes32 public constant OFFER_TYPEHASH = keccak256(
        "Offer(address seller,address buyer,address token,uint256 amount,uint8 feePayer,uint16 maxFeeBps,uint256 maxBaseGas,uint256 maxClaimGas,uint256 maxMinFee,uint256 bond,uint16 lateSlashBps,uint8 interval,uint32 periods,uint32 startWindow,bytes32 listingHash,uint64 expiry,uint32 maxFills,uint256 nonce)"
    );

    uint256 internal constant BPS = 10_000;

    uint256 public constant MIN_START_WINDOW = 1 days;
    uint256 public constant MAX_START_WINDOW = 7 days;
    uint256 public constant MAX_DURATION = 365 days;
    uint256 public constant MIN_PERIOD_AMOUNT = 100_000; // 0.1 USDT of net principal per period
    uint256 public constant CLAIM_INTERVAL = 24 hours;
    uint256 public constant MAX_ARBITER_EXTENSION_CALL = 7 days;
    uint256 public constant MAX_ARBITER_EXTENSION = 14 days;

    Escrow public immutable escrow; // seller bond pool
    EscrowConfig public immutable config;
    address public immutable arbiter;
    address public immutable feeRecipient;

    uint256 public orderCount;
    mapping(uint256 => Order) internal _orders;

    event OrderCreated(
        uint256 indexed orderId,
        bytes32 indexed offerHash,
        address indexed buyer,
        address seller,
        uint256 amount,
        bytes32 listingHash,
        uint32 interval,
        uint32 periods
    );
    event Started(uint256 indexed orderId, uint64 startedAt, uint64 endsAt);
    event Claimed(uint256 indexed orderId, uint256 toSeller, uint256 claimGas);
    event DeadlineExtended(uint256 indexed orderId, uint64 deadline);
    event EndProposed(uint256 indexed orderId, address indexed proposer, uint256 toBuyer, uint64 expiry);
    event Disputed(uint256 indexed orderId, address indexed by, uint256 disputeFee, FeeSource source);
    event Resolved(uint256 indexed orderId, uint256 toBuyerBps, uint256 bondToBuyerBps);
    event Settled(uint256 indexed orderId, State state, uint256 toBuyer, uint256 toSeller, uint256 toFeeRecipient);

    error BadOffer();
    error FeeLimit();
    error FeesExceedAmount();
    error BadState();
    error NotParty();
    error TooEarly();
    error TooLate();
    error BadParams();
    error NothingToClaim();

    /// @dev Arbiter, fee recipient and config are Escrow's, so both markets always share them.
    constructor(Escrow escrow_) EIP712("AkariStreamEscrow", "1") {
        if (address(escrow_) == address(0)) revert BadParams();
        escrow = escrow_;
        config = escrow_.config();
        arbiter = escrow_.arbiter();
        feeRecipient = escrow_.feeRecipient();
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
        // Campaigns scoped to "escrow" cover both escrow modes (one-time and installment).
        EscrowConfig.Quote memory q = config.checkout(offer.token, config.SCOPE_ESCROW());
        _checkTerms(offer, q);
        orderId = ++orderCount;
        uint256 buyerPays = _record(orderId, offer, q);
        if (offer.bond > 0) escrow.lockBond(offer.seller, offer.token, offer.bond);
        config.lockFunds(buyerPays, true);
        emit OrderCreated(
            orderId,
            offerHash,
            msg.sender,
            offer.seller,
            offer.amount,
            offer.listingHash,
            _orders[orderId].interval,
            offer.periods
        );
        _pullExact(offer.token, buyerPays);
    }

    /// @notice Seller starts the service before the start deadline; unlocking begins now.
    function start(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (msg.sender != o.seller) revert NotParty();
        if (o.startedAt != 0) revert BadState();
        if (block.timestamp > o.startDeadline) revert TooLate();
        o.platformPaid = true;
        o.startedAt = uint64(block.timestamp);
        emit Started(orderId, uint64(block.timestamp), uint64(_endsAt(o)));
    }

    /// @notice Seller claims the unlocked, unclaimed principal minus the claim gas fee. Needs more than the claim gas
    ///         fee to claim and 24 hours since the last claim, so every fee charged matches one sponsored claim.
    ///         After the stream has ended this settles the order instead (same as `finalize`).
    function claim(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (msg.sender != o.seller) revert NotParty();
        if (o.startedAt == 0) revert BadState();
        if (block.timestamp >= _endsAt(o)) {
            _settle(orderId, State.Released, o.net, 0, 0); // stream over: same as `finalize`, no claim gas
            return;
        }
        if (o.lastClaimAt != 0 && block.timestamp < uint256(o.lastClaimAt) + CLAIM_INTERVAL) revert TooEarly();
        uint256 unlockedNow = _unlocked(o);
        uint256 gas = o.claimGas;
        uint256 toSeller = FeeMath.claimNet(unlockedNow, o.claimed, gas);
        if (toSeller == 0) revert NothingToClaim();
        o.claimed = uint128(unlockedNow);
        o.lastClaimAt = uint64(block.timestamp);
        config.unlockFunds(toSeller + gas);
        emit Claimed(orderId, toSeller, gas);
        address token = o.token;
        _push(token, o.seller, toSeller);
        _push(token, feeRecipient, gas);
    }

    /// @notice Anyone (the platform worker), once the stream has ended: pay the seller the remainder and settle.
    ///         No claim gas: the base gas already covers the final settlement.
    function finalize(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (o.startedAt == 0) revert BadState();
        if (block.timestamp < _endsAt(o)) revert TooEarly();
        _settle(orderId, State.Released, o.net, 0, 0);
    }

    /// @notice Seller voluntarily refunds everything not yet claimed. The bond is not slashed; fees are charged only
    ///         on what the seller already claimed.
    function refund(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (msg.sender != o.seller) revert NotParty();
        _settle(orderId, State.Refunded, o.claimed, 0, 0);
    }

    /// @notice Anyone: full refund when the seller did not start by the start deadline (then `lateSlashBps` of the
    ///         bond goes to the buyer).
    function claimTimeout(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (o.startedAt != 0) revert BadState();
        if (block.timestamp <= o.startDeadline) revert TooEarly();
        _settle(orderId, State.Refunded, 0, 0, o.lateSlashBps);
    }

    /// @notice Arbiter Safe, when the main site is down: extend not-started orders' start deadlines. Never moves
    ///         funds. ≤ 7 days per call and ≤ 14 days per order in total; other orders are skipped.
    function extendDeadlines(uint256[] calldata orderIds, uint32 secs) external nonReentrant {
        if (msg.sender != arbiter) revert NotParty();
        if (secs == 0 || secs > MAX_ARBITER_EXTENSION_CALL) revert BadParams();
        for (uint256 i; i < orderIds.length; ++i) {
            Order storage o = _orders[orderIds[i]];
            uint256 total = uint256(o.arbExtended) + secs;
            if (o.state != State.Funded || o.startedAt != 0 || total > MAX_ARBITER_EXTENSION) continue;
            o.arbExtended = uint32(total);
            o.startDeadline += secs;
            emit DeadlineExtended(orderIds[i], o.startDeadline);
        }
    }

    /// @notice Either party proposes how much of the unclaimed principal goes back to the buyer (the UI suggests
    ///         "unlocked to the seller, the rest to the buyer"); replaces any earlier proposal.
    function proposeEnd(uint256 orderId, uint256 toBuyer) external nonReentrant {
        Order storage o = _open(orderId);
        if (msg.sender != o.buyer && msg.sender != o.seller) revert NotParty();
        if (toBuyer > o.net - o.claimed) revert BadParams();
        o.platformPaid = true;
        o.proposer = msg.sender;
        o.proposalToBuyer = uint128(toBuyer);
        o.proposalExpiry = uint64(block.timestamp + o.proposalWindow);
        emit EndProposed(orderId, msg.sender, toBuyer, o.proposalExpiry);
    }

    /// @notice The counterparty accepts the live proposal. `toBuyer` must match (guards against a swapped proposal)
    ///         and must still fit in the unclaimed principal. Accepting during a dispute refunds the dispute fee.
    function acceptEnd(uint256 orderId, uint256 toBuyer) external nonReentrant {
        Order storage o = _open(orderId);
        if (msg.sender != o.buyer && msg.sender != o.seller) revert NotParty();
        if (o.proposer == address(0) || o.proposer == msg.sender) revert BadState();
        if (block.timestamp > o.proposalExpiry) revert TooLate();
        if (toBuyer != o.proposalToBuyer || toBuyer > o.net - o.claimed) revert BadParams();
        o.platformPaid = true;
        _settle(orderId, State.Settled, o.net - toBuyer, 0, 0);
    }

    /// @notice Either party opens a dispute (before the start deadline, or before the stream ends) and prepays the
    ///         dispute fee. A seller pays from this order's unlocked, unclaimed principal when it covers the fee,
    ///         otherwise from the wallet. Unlocking freezes at `disputedAt`.
    function dispute(uint256 orderId) external nonReentrant {
        _dispute(orderId, false);
    }

    /// @notice Seller opens a dispute paying the dispute fee from their available bond in Escrow.
    function disputeWithBond(uint256 orderId) external nonReentrant {
        _dispute(orderId, true);
    }

    /// @notice Arbiter Safe executes a published ruling over everything not yet claimed (including principal that
    ///         unlocked before the dispute): `toBuyerBps` of it to the buyer, the rest to the seller, and
    ///         `bondToBuyerBps` of the locked bond to the buyer (the rest is unlocked for the seller).
    function resolve(uint256 orderId, uint256 toBuyerBps, uint256 bondToBuyerBps) external nonReentrant {
        if (msg.sender != arbiter) revert NotParty();
        Order storage o = _disputed(orderId);
        if (toBuyerBps > BPS || bondToBuyerBps > BPS) revert BadParams();
        uint256 disputerWinBps = o.sellerDisputed ? BPS - toBuyerBps : toBuyerBps;
        uint256 retained = uint256(o.disputeFee) * (BPS - disputerWinBps) / BPS;
        uint256 unclaimed = o.net - o.claimed;
        emit Resolved(orderId, toBuyerBps, bondToBuyerBps);
        _settle(orderId, State.Settled, o.claimed + unclaimed * (BPS - toBuyerBps) / BPS, retained, bondToBuyerBps);
    }

    /// @notice Anyone, once a dispute has gone unresolved for `resolveTimeout`: never started → full refund;
    ///         started → the seller keeps what had unlocked when the dispute was opened and the rest is split 50/50.
    ///         Dispute fee refunded in full; bond unlocked.
    function resolveTimeout(uint256 orderId) external nonReentrant {
        Order storage o = _disputed(orderId);
        if (block.timestamp <= uint256(o.disputedAt) + o.resolveTimeout) revert TooEarly();
        uint256 atDispute = _unlocked(o); // frozen at disputedAt
        _settle(orderId, State.Settled, o.startedAt == 0 ? 0 : atDispute + (o.net - atDispute) / 2, 0, 0);
    }

    // ───────────────────────────── Views ─────────────────────────────

    function getOrder(uint256 orderId) external view returns (Order memory) {
        return _orders[orderId];
    }

    /// @notice Net principal unlocked so far (frozen at `disputedAt` once disputed). Meaningful while open.
    function unlocked(uint256 orderId) external view returns (uint256) {
        return _unlocked(_orders[orderId]);
    }

    /// @dev The struct contains only static types, so `abi.encode(typehash, o)` is exactly EIP-712 `encodeData`.
    function hashOffer(Offer calldata o) public view returns (bytes32) {
        return _hashTypedDataV4(keccak256(abi.encode(OFFER_TYPEHASH, o)));
    }

    function intervalSeconds(Interval i) public pure returns (uint32) {
        if (i == Interval.Hour) return 1 hours;
        if (i == Interval.Day) return 1 days;
        if (i == Interval.Week) return 7 days;
        return 30 days;
    }

    // ───────────────────────────── Internal ─────────────────────────────

    /// @dev Writes the new order with its parameter snapshot; returns what the buyer must pay.
    function _record(uint256 orderId, Offer calldata offer, EscrowConfig.Quote memory q) internal returns (uint256) {
        uint256 feeTotal = FeeMath.feeTotal(offer.amount, q.feeBps, q.minFee, q.campaign);
        (uint256 buyerFee, uint256 sellerFee) = FeeMath.split(feeTotal, offer.feePayer);
        (uint256 buyerGas, uint256 sellerGas) = FeeMath.split(q.baseGas, offer.feePayer);
        if (sellerFee + sellerGas >= offer.amount) revert FeesExceedAmount();
        uint256 net = FeeMath.streamNet(offer.amount, sellerFee, sellerGas);
        if (net < uint256(offer.periods) * MIN_PERIOD_AMOUNT) revert BadOffer();

        Order storage o = _orders[orderId];
        o.buyer = msg.sender;
        o.seller = offer.seller;
        o.token = offer.token;
        o.state = State.Funded;
        o.net = uint128(net);
        o.buyerFee = uint128(buyerFee);
        o.sellerFee = uint128(sellerFee);
        o.buyerGas = uint128(buyerGas);
        o.sellerGas = uint128(sellerGas);
        o.claimGas = uint128(q.claimGas);
        o.disputeFee = uint128(FeeMath.disputeFee(feeTotal, q.disputeMinFee, q.disputeMulBps));
        o.bond = uint128(offer.bond);
        o.lateSlashBps = offer.lateSlashBps;
        o.interval = intervalSeconds(offer.interval);
        o.periods = offer.periods;
        o.startDeadline = uint64(block.timestamp + offer.startWindow);
        o.resolveTimeout = q.resolveTimeout;
        o.proposalWindow = q.proposalWindow;
        return FeeMath.buyerPays(offer.amount, buyerFee, buyerGas);
    }

    function _dispute(uint256 orderId, bool fromBond) internal {
        Order storage o = _funded(orderId);
        bool bySeller = msg.sender == o.seller;
        if (!bySeller && msg.sender != o.buyer) revert NotParty();
        if (fromBond && !bySeller) revert NotParty();
        if (o.startedAt == 0 ? block.timestamp > o.startDeadline : block.timestamp >= _endsAt(o)) revert TooLate();
        uint256 fee = o.disputeFee;
        FeeSource source = FeeSource.Wallet;
        if (fromBond) source = FeeSource.Bond;
        else if (bySeller && _unlocked(o) - o.claimed >= fee) source = FeeSource.Unlocked;
        o.state = State.Disputed;
        o.sellerDisputed = bySeller;
        o.disputedAt = uint64(block.timestamp);
        o.disputeSource = source;
        emit Disputed(orderId, msg.sender, fee, source);
        if (source == FeeSource.Unlocked) {
            o.claimed += uint128(fee); // principal moves into the dispute fee: total custody is unchanged
        } else if (source == FeeSource.Bond) {
            escrow.takeBond(o.seller, o.token, fee); // moves from the bond pool to this market: custody unchanged
        } else {
            config.lockFunds(fee, false); // never block a dispute on the global cap
            _pullExact(o.token, fee);
        }
    }

    function _settle(uint256 orderId, State end, uint256 s, uint256 disputeRetained, uint256 bondSlashBps) internal {
        Order storage o = _orders[orderId];
        (uint256 toBuyer, uint256 toSeller, uint256 toFeeRecipient) = _shares(o, s, disputeRetained);
        if (o.disputedAt != 0) {
            // Whatever the source, a refunded seller dispute fee is paid to the seller's wallet.
            uint256 disputeRefund = o.disputeFee - disputeRetained;
            if (o.sellerDisputed) toSeller += disputeRefund;
            else toBuyer += disputeRefund;
        }

        o.state = end;
        config.unlockFunds(toBuyer + toSeller + toFeeRecipient);
        emit Settled(orderId, end, toBuyer, toSeller, toFeeRecipient);
        address token = o.token;
        address buyer = o.buyer;
        uint256 bond = o.bond;
        if (bond > 0) escrow.releaseBond(o.seller, token, bond, buyer, bond * bondSlashBps / BPS);
        _push(token, buyer, toBuyer);
        _push(token, o.seller, toSeller);
        _push(token, feeRecipient, toFeeRecipient);
    }

    /// @dev Splits what the order still holds (unclaimed principal + fee pool) for seller share `s` of `net`.
    ///      A retained dispute fee goes to the fee recipient and replaces the platform fee (at most one is charged).
    function _shares(Order storage o, uint256 s, uint256 disputeRetained)
        internal
        view
        returns (uint256 toBuyer, uint256 toSeller, uint256 toFeeRecipient)
    {
        uint256 net = o.net;
        (uint256 feeB, uint256 feeS) = FeeMath.feesCharged(o.buyerFee, o.sellerFee, s, net, disputeRetained);
        uint256 pool = uint256(o.buyerFee) + o.sellerFee + o.buyerGas + o.sellerGas;
        uint256 gas = o.platformPaid ? uint256(o.buyerGas) + o.sellerGas : 0;
        toSeller = s - o.claimed;
        toFeeRecipient = gas + feeB + feeS + disputeRetained;
        toBuyer = net - s + pool + disputeRetained - toFeeRecipient;
    }

    /// @dev Unlocked net principal at min(now, disputedAt); whole periods only, the remainder lands in the last one.
    function _unlocked(Order storage o) internal view returns (uint256) {
        uint256 startedAt = o.startedAt;
        if (startedAt == 0) return 0;
        uint256 t = o.disputedAt != 0 ? o.disputedAt : block.timestamp;
        return FeeMath.streamUnlocked(o.net, (t - startedAt) / o.interval, o.periods);
    }

    function _endsAt(Order storage o) internal view returns (uint256) {
        return uint256(o.startedAt) + uint256(o.interval) * o.periods;
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

    function _checkTerms(Offer calldata o, EscrowConfig.Quote memory q) internal view {
        if (o.seller == address(0) || o.seller == msg.sender) revert BadOffer();
        if (o.amount < q.minOrder || o.amount > q.orderCap) revert BadOffer();
        if (o.bond > o.amount || o.lateSlashBps > BPS) revert BadOffer();
        if (o.startWindow < MIN_START_WINDOW || o.startWindow > MAX_START_WINDOW) revert BadOffer();
        if (o.periods == 0 || uint256(o.periods) * intervalSeconds(o.interval) > MAX_DURATION) revert BadOffer();
        if (q.feeBps > o.maxFeeBps || q.baseGas > o.maxBaseGas || q.claimGas > o.maxClaimGas || q.minFee > o.maxMinFee)
        {
            revert FeeLimit();
        }
    }
}
