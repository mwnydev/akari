// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {ReentrancyGuardTransient} from "@openzeppelin/contracts/utils/ReentrancyGuardTransient.sol";
import {EIP712} from "@openzeppelin/contracts/utils/cryptography/EIP712.sol";
import {SignatureCheckerLib} from "solady/utils/SignatureCheckerLib.sol";
import {FeeMath} from "./libraries/FeeMath.sol";

/// @title Escrow — one-time USDT escrow (plan-v3 §6)
/// @notice Funds can only flow to the order's buyer, seller, and the fee recipient (fees, base gas, dispute fees).
///         No admin withdrawal, no proxy, no selfdestruct. `pause` only blocks new orders.
/// @dev Settlement model: every ending is expressed as `s` = the seller's share of the order principal (`amount`).
///      The fee pool (fees + base gas) is kept apart from principal; fees are earned pro rata to `s`, base gas is
///      earned in full once the platform has sponsored any post-payment operation (`platformPaid`).
contract Escrow is Ownable2Step, ReentrancyGuardTransient, EIP712 {
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
        uint256 amount;
        FeeMath.Payer feePayer;
        uint16 maxFeeBps;
        uint256 maxBaseGas;
        uint256 maxMinFee; // highest minimum fee the seller accepts (0 applies during a campaign)
        bool instant;
        uint32 shipWindow;
        uint32 confirmWindow;
        bytes32 listingHash;
        uint64 expiry;
        uint32 maxFills;
        uint256 nonce;
    }

    struct Params {
        uint16 feeBps;
        uint16 campaignBps;
        bool campaign; // campaign rate replaces feeBps and disables minFee
        uint64 minFee;
        uint64 minOrder;
        uint64 orderCap;
        uint64 baseGas;
        uint32 disputeMulBps;
        uint32 resolveTimeout;
        uint32 proposalWindow;
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
        uint128 amount;
        uint128 disputeFee;
        uint128 buyerFee;
        uint128 sellerFee;
        uint128 buyerGas;
        uint128 sellerGas;
        uint64 disputedAt;
        uint32 resolveTimeout;
        uint32 sellerExtended;
        uint32 proposalWindow;
        address proposer;
        uint64 proposalExpiry;
        uint128 proposalToBuyer;
    }

    bytes32 public constant OFFER_TYPEHASH = keccak256(
        "Offer(address seller,address buyer,uint256 amount,uint8 feePayer,uint16 maxFeeBps,uint256 maxBaseGas,uint256 maxMinFee,bool instant,uint32 shipWindow,uint32 confirmWindow,bytes32 listingHash,uint64 expiry,uint32 maxFills,uint256 nonce)"
    );

    uint256 internal constant BPS = 10_000;
    uint256 internal constant USDT = 1e6;

    // Hard caps (plan §9). The owner Safe can never set values outside these.
    uint256 public constant MAX_FEE_BPS = 1000;
    uint256 public constant MAX_MIN_FEE = 5 * USDT;
    uint256 public constant MIN_MIN_ORDER = 1 * USDT;
    uint256 public constant MAX_MIN_ORDER = 100 * USDT;
    uint256 public constant MAX_ORDER_CAP = 100_000 * USDT;
    uint256 public constant MAX_BASE_GAS = 1 * USDT;
    uint256 public constant MIN_DISPUTE_MUL = 5000;
    uint256 public constant MAX_DISPUTE_MUL = 50_000;
    uint256 public constant MIN_RESOLVE_TIMEOUT = 14 days;
    uint256 public constant MAX_RESOLVE_TIMEOUT = 60 days;
    uint256 public constant MIN_PROPOSAL_WINDOW = 1 days;
    uint256 public constant MAX_PROPOSAL_WINDOW = 7 days;
    uint256 public constant MIN_WINDOW = 1 days;
    uint256 public constant MAX_WINDOW = 30 days;
    uint256 public constant MAX_SELLER_EXTENSION = 30 days;
    uint256 public constant EMERGENCY_WINDOW = 6 hours;
    uint256 public constant EMERGENCY_EXTENSION = 24 hours;
    uint256 public constant PARAM_DELAY = 72 hours;

    IERC20 public immutable token;
    address public immutable arbiter;
    address public immutable feeRecipient;

    Params internal _params;
    Params internal _pending;
    uint64 public pendingEta; // 0 = nothing queued
    bool public paused;

    uint256 public orderCount;
    // slither-disable-next-line uninitialized-state (written through a storage pointer in `_record`)
    mapping(uint256 => Order) internal _orders;
    mapping(bytes32 => uint256) public fills;
    mapping(address => mapping(uint256 => bool)) public cancelledNonce;
    mapping(address => uint256) public credit;

    event OrderCreated(
        uint256 indexed orderId,
        bytes32 indexed offerHash,
        address indexed buyer,
        address seller,
        uint256 amount,
        bytes32 listingHash,
        bool instant
    );
    event Delivered(uint256 indexed orderId, uint64 confirmDeadline);
    event ConfirmExtended(uint256 indexed orderId, address indexed by, uint64 confirmDeadline);
    event EndProposed(uint256 indexed orderId, address indexed proposer, uint256 toBuyer, uint64 expiry);
    event Disputed(uint256 indexed orderId, address indexed by, uint256 disputeFee);
    event Resolved(uint256 indexed orderId, uint256 toBuyerBps);
    event Settled(uint256 indexed orderId, State state, uint256 toBuyer, uint256 toSeller, uint256 toFeeRecipient);
    event Credited(address indexed account, uint256 amount);
    event Withdrawn(address indexed account, address indexed to, uint256 amount);
    event OfferCancelled(address indexed seller, uint256 nonce);
    event ParamsQueued(Params params, uint64 eta);
    event ParamsApplied(Params params);
    event ParamsCancelled();
    event PausedSet(bool paused);

    error IsPaused();
    error BadOffer();
    error BadSignature();
    error OfferUnavailable();
    error FeeLimit();
    error FeesExceedAmount();
    error AmountMismatch();
    error BadState();
    error NotParty();
    error TooEarly();
    error TooLate();
    error BadParams();
    error NothingToWithdraw();

    constructor(IERC20 token_, address owner_, address arbiter_, address feeRecipient_, Params memory params_)
        Ownable(owner_)
        EIP712("AkariEscrow", "1")
    {
        if (address(token_) == address(0) || arbiter_ == address(0) || feeRecipient_ == address(0)) {
            revert BadParams();
        }
        _checkParams(params_);
        token = token_;
        arbiter = arbiter_;
        feeRecipient = feeRecipient_;
        _params = params_;
        emit ParamsApplied(params_);
    }

    // ───────────────────────────── Orders ─────────────────────────────

    /// @notice Buyer verifies the seller's signed offer, opens the order and pays in one call.
    function createAndFund(Offer calldata offer, bytes calldata sellerSig)
        external
        nonReentrant
        returns (uint256 orderId)
    {
        if (paused) revert IsPaused();
        Params memory p = _params;
        _checkOffer(offer, p);

        bytes32 offerHash = hashOffer(offer);
        if (fills[offerHash] >= offer.maxFills) revert OfferUnavailable();
        fills[offerHash] += 1;
        if (!SignatureCheckerLib.isValidERC6492SignatureNow(offer.seller, offerHash, sellerSig)) {
            revert BadSignature();
        }

        orderId = ++orderCount;
        uint256 buyerPays = _record(orderId, offer, p);
        emit OrderCreated(orderId, offerHash, msg.sender, offer.seller, offer.amount, offer.listingHash, offer.instant);
        if (offer.instant) emit Delivered(orderId, _orders[orderId].confirmDeadline);
        _pullExact(buyerPays);
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
        _settle(orderId, State.Released, o.amount, 0);
    }

    /// @notice Seller voluntarily refunds in full. No fee is charged.
    function refund(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (msg.sender != o.seller) revert NotParty();
        _settle(orderId, State.Refunded, 0, 0);
    }

    /// @notice Anyone: release after the confirm window, or refund when the seller missed the ship deadline.
    function claimTimeout(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        if (o.confirmDeadline != 0) {
            if (block.timestamp <= o.confirmDeadline) revert TooEarly();
            o.platformPaid = true; // the worker submits this release on the platform's gas
            _settle(orderId, State.Released, o.amount, 0);
        } else {
            if (block.timestamp <= o.shipDeadline) revert TooEarly();
            _settle(orderId, State.Refunded, 0, 0);
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
        _settle(orderId, State.Settled, o.amount - toBuyer, 0);
    }

    /// @notice Either party opens a dispute before the active deadline and prepays the dispute fee.
    function dispute(uint256 orderId) external nonReentrant {
        Order storage o = _funded(orderId);
        bool bySeller = msg.sender == o.seller;
        if (!bySeller && msg.sender != o.buyer) revert NotParty();
        uint256 deadline = o.confirmDeadline != 0 ? o.confirmDeadline : o.shipDeadline;
        if (block.timestamp > deadline) revert TooLate();
        o.state = State.Disputed;
        o.sellerDisputed = bySeller;
        o.disputedAt = uint64(block.timestamp);
        emit Disputed(orderId, msg.sender, o.disputeFee);
        _pullExact(o.disputeFee);
    }

    /// @notice Arbiter Safe executes a published ruling: `toBuyerBps` of the principal to the buyer, rest to seller.
    function resolve(uint256 orderId, uint256 toBuyerBps) external nonReentrant {
        if (msg.sender != arbiter) revert NotParty();
        Order storage o = _disputed(orderId);
        if (toBuyerBps > BPS) revert BadParams();
        uint256 disputerWinBps = o.sellerDisputed ? BPS - toBuyerBps : toBuyerBps;
        uint256 retained = uint256(o.disputeFee) * (BPS - disputerWinBps) / BPS;
        emit Resolved(orderId, toBuyerBps);
        _settle(orderId, State.Settled, uint256(o.amount) * (BPS - toBuyerBps) / BPS, retained);
    }

    /// @notice Anyone, once a dispute has gone unresolved for `resolveTimeout`: not delivered → full refund;
    ///         delivered → principal split 50/50. Dispute fee refunded in full.
    function resolveTimeout(uint256 orderId) external nonReentrant {
        Order storage o = _disputed(orderId);
        if (block.timestamp <= uint256(o.disputedAt) + o.resolveTimeout) revert TooEarly();
        _settle(orderId, State.Settled, o.confirmDeadline != 0 ? o.amount / 2 : 0, 0);
    }

    /// @notice Withdraw a balance that could not be pushed (e.g. the token froze the recipient) to another address.
    function withdraw(address to) external nonReentrant {
        uint256 amt = credit[msg.sender];
        if (amt == 0) revert NothingToWithdraw();
        credit[msg.sender] = 0;
        emit Withdrawn(msg.sender, to, amt);
        token.safeTransfer(to, amt);
    }

    /// @notice Seller voids a signed offer by nonce.
    function cancelOffer(uint256 nonce) external {
        cancelledNonce[msg.sender][nonce] = true;
        emit OfferCancelled(msg.sender, nonce);
    }

    // ───────────────────────────── Owner (treasury Safe) ─────────────────────────────

    /// @notice Loosening changes apply now; anything that raises charges or tightens rules waits 72 hours.
    function setParams(Params calldata p) external onlyOwner {
        _checkParams(p);
        if (_isLoosening(p, _params)) {
            _params = p;
            emit ParamsApplied(p);
        } else {
            _pending = p;
            pendingEta = uint64(block.timestamp + PARAM_DELAY);
            emit ParamsQueued(p, pendingEta);
        }
    }

    /// @notice Anyone applies the queued params once the delay has passed.
    function applyParams() external {
        if (pendingEta == 0) revert BadState();
        if (block.timestamp < pendingEta) revert TooEarly();
        _params = _pending;
        pendingEta = 0;
        emit ParamsApplied(_params);
    }

    function cancelParams() external onlyOwner {
        if (pendingEta == 0) revert BadState();
        pendingEta = 0;
        emit ParamsCancelled();
    }

    /// @notice Pausing only blocks new orders; existing orders keep every exit.
    function setPaused(bool paused_) external onlyOwner {
        paused = paused_;
        emit PausedSet(paused_);
    }

    // ───────────────────────────── Views ─────────────────────────────

    function params() external view returns (Params memory) {
        return _params;
    }

    function pendingParams() external view returns (Params memory) {
        return _pending;
    }

    function getOrder(uint256 orderId) external view returns (Order memory) {
        return _orders[orderId];
    }

    /// @dev Every Offer field is a static type, so `abi.encode(typehash, offer)` is exactly EIP-712 `encodeData`.
    function hashOffer(Offer calldata o) public view returns (bytes32) {
        return _hashTypedDataV4(keccak256(abi.encode(OFFER_TYPEHASH, o)));
    }

    // ───────────────────────────── Internal ─────────────────────────────

    /// @dev Writes the new order with its parameter snapshot; returns what the buyer must pay.
    function _record(uint256 orderId, Offer calldata offer, Params memory p) internal returns (uint256) {
        uint256 feeTotal = FeeMath.feeTotal(offer.amount, p.campaign ? p.campaignBps : p.feeBps, p.minFee, p.campaign);
        (uint256 buyerFee, uint256 sellerFee) = FeeMath.split(feeTotal, offer.feePayer);
        (uint256 buyerGas, uint256 sellerGas) = FeeMath.split(p.baseGas, offer.feePayer);
        if (sellerFee + sellerGas >= offer.amount) revert FeesExceedAmount();

        Order storage o = _orders[orderId];
        o.buyer = msg.sender;
        o.seller = offer.seller;
        o.state = State.Funded;
        o.amount = uint128(offer.amount);
        o.buyerFee = uint128(buyerFee);
        o.sellerFee = uint128(sellerFee);
        o.buyerGas = uint128(buyerGas);
        o.sellerGas = uint128(sellerGas);
        o.disputeFee = uint128(FeeMath.disputeFee(feeTotal, p.minFee, p.disputeMulBps));
        o.confirmWindow = offer.confirmWindow;
        o.resolveTimeout = p.resolveTimeout;
        o.proposalWindow = p.proposalWindow;
        if (offer.instant) {
            o.confirmDeadline = uint64(block.timestamp + offer.confirmWindow);
        } else {
            o.shipDeadline = uint64(block.timestamp + offer.shipWindow);
        }
        return FeeMath.buyerPays(offer.amount, buyerFee, buyerGas);
    }

    function _settle(uint256 orderId, State end, uint256 s, uint256 disputeRetained) internal {
        Order storage o = _orders[orderId];
        uint256 amount = o.amount;
        bool gasEarned = o.platformPaid;

        uint256 feeB = uint256(o.buyerFee) * s / amount;
        uint256 feeS = uint256(o.sellerFee) * s / amount;
        uint256 fee = feeB + feeS;
        if (disputeRetained > 0 && fee > 0) {
            // A retained dispute fee replaces the platform fee: charge at most one of the two.
            // Scaling the floored integer feeB (not the exact ratio) keeps feeB' <= feeB and feeS' <= feeS,
            // which is what guarantees `toBuyer` below cannot underflow.
            uint256 charged = fee > disputeRetained ? fee - disputeRetained : 0;
            // slither-disable-next-line divide-before-multiply
            feeB = feeB * charged / fee;
            feeS = charged - feeB;
            fee = charged;
        }
        uint256 gas = gasEarned ? uint256(o.buyerGas) + o.sellerGas : 0;
        uint256 sellerCost = feeS + (gasEarned ? o.sellerGas : 0);
        uint256 toSeller = s > sellerCost ? s - sellerCost : 0;
        uint256 toBuyer = amount + o.buyerFee + o.buyerGas - gas - fee - toSeller;
        uint256 toFeeRecipient = gas + fee + disputeRetained;
        if (o.disputedAt != 0) {
            uint256 disputeRefund = o.disputeFee - disputeRetained;
            if (o.sellerDisputed) toSeller += disputeRefund;
            else toBuyer += disputeRefund;
        }

        o.state = end;
        emit Settled(orderId, end, toBuyer, toSeller, toFeeRecipient);
        _push(o.buyer, toBuyer);
        _push(o.seller, toSeller);
        _push(feeRecipient, toFeeRecipient);
    }

    /// @dev Push payment; if the token refuses (frozen address), keep it claimable via `withdraw(to)`.
    function _push(address to, uint256 amt) internal {
        if (amt == 0) return;
        if (!token.trySafeTransfer(to, amt)) {
            credit[to] += amt;
            emit Credited(to, amt);
        }
    }

    /// @dev Pull exactly `amt` from the caller, measured by balance change (never trust the transfer's accounting).
    function _pullExact(uint256 amt) internal {
        uint256 before = token.balanceOf(address(this));
        token.safeTransferFrom(msg.sender, address(this), amt);
        if (token.balanceOf(address(this)) - before != amt) revert AmountMismatch();
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

    function _checkOffer(Offer calldata offer, Params memory p) internal view {
        if (offer.seller == address(0) || offer.seller == msg.sender) revert BadOffer();
        if (offer.buyer != address(0) && offer.buyer != msg.sender) revert BadOffer();
        if (block.timestamp > offer.expiry || cancelledNonce[offer.seller][offer.nonce]) revert OfferUnavailable();
        if (offer.amount < p.minOrder || offer.amount > p.orderCap) revert BadOffer();
        if (offer.confirmWindow < MIN_WINDOW || offer.confirmWindow > MAX_WINDOW) revert BadOffer();
        if (!offer.instant && (offer.shipWindow < MIN_WINDOW || offer.shipWindow > MAX_WINDOW)) revert BadOffer();
        if (
            (p.campaign ? p.campaignBps : p.feeBps) > offer.maxFeeBps || p.baseGas > offer.maxBaseGas
                || (p.campaign ? 0 : p.minFee) > offer.maxMinFee
        ) {
            revert FeeLimit();
        }
    }

    function _checkParams(Params memory p) internal pure {
        if (
            p.feeBps > MAX_FEE_BPS || p.campaignBps > MAX_FEE_BPS || p.minFee > MAX_MIN_FEE
                || p.minOrder < MIN_MIN_ORDER || p.minOrder > MAX_MIN_ORDER || p.orderCap < p.minOrder
                || p.orderCap > MAX_ORDER_CAP || p.baseGas > MAX_BASE_GAS || p.disputeMulBps < MIN_DISPUTE_MUL
                || p.disputeMulBps > MAX_DISPUTE_MUL || p.resolveTimeout < MIN_RESOLVE_TIMEOUT
                || p.resolveTimeout > MAX_RESOLVE_TIMEOUT || p.proposalWindow < MIN_PROPOSAL_WINDOW
                || p.proposalWindow > MAX_PROPOSAL_WINDOW
        ) revert BadParams();
    }

    /// @dev True when no field raises a charge or tightens a rule. Timeouts and windows are treated as rules:
    ///      any change to them is queued. Ending a campaign re-enables the minimum fee, so it is queued too.
    function _isLoosening(Params memory n, Params memory c) internal pure returns (bool) {
        bool campaignOk = n.campaign ? n.campaignBps <= (c.campaign ? c.campaignBps : c.feeBps) : !c.campaign;
        return campaignOk && n.feeBps <= c.feeBps && n.minFee <= c.minFee && n.minOrder <= c.minOrder
            && n.orderCap >= c.orderCap && n.baseGas <= c.baseGas && n.disputeMulBps <= c.disputeMulBps
            && n.resolveTimeout == c.resolveTimeout && n.proposalWindow == c.proposalWindow;
    }
}
