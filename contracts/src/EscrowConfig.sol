// SPDX-License-Identifier: MIT
pragma solidity 0.8.37;

import {Ownable, Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";

/// @title EscrowConfig — owner-controlled settings shared by Escrow and StreamEscrow (plan-v3 §6, §9)
/// @notice Owner = treasury Safe. Every value is bounded by hard caps below. Anything that raises charges or
///         tightens rules waits 72 hours (queued, cancellable, then applied by anyone); loosening applies at once.
///         Markets (Escrow, StreamEscrow) read a snapshot through `checkout` when an order is created, so changes
///         here never touch existing orders.
contract EscrowConfig is Ownable2Step {
    struct Params {
        uint16 feeBps;
        uint64 minFee;
        uint64 minOrder;
        uint64 orderCap;
        uint64 totalLockedCap; // cap on everything held by all markets (open orders + seller bonds)
        uint64 baseGas;
        uint32 disputeMulBps;
        uint32 resolveTimeout;
        uint32 proposalWindow;
    }

    /// @dev `end == 0` means no campaign. `scope` is a bitmask of SCOPE_ESCROW / SCOPE_AUCTION.
    struct Campaign {
        uint64 start;
        uint64 end;
        uint16 rateBps;
        uint8 scope;
        uint32 gasSubsidyCap; // orders whose base gas the platform waives (0 = fees only)
    }

    /// @notice Parameter snapshot handed to a market for one new order.
    struct Quote {
        uint256 feeBps; // effective rate (campaign rate if a campaign applies)
        uint256 minFee; // effective minimum fee (0 during a campaign)
        bool campaign;
        uint256 baseGas; // 0 when this order uses a gas-subsidy slot
        uint256 disputeMinFee; // dispute-fee floor for zero-fee orders: the regular minimum fee
        uint256 minOrder;
        uint256 orderCap;
        uint256 disputeMulBps;
        uint32 resolveTimeout;
        uint32 proposalWindow;
    }

    uint256 internal constant USDT = 1e6;
    uint8 public constant SCOPE_ESCROW = 1;
    uint8 public constant SCOPE_AUCTION = 2;

    // Hard caps (plan §9). The owner Safe can never set values outside these.
    uint256 public constant MAX_FEE_BPS = 1000;
    uint256 public constant MAX_MIN_FEE = 5 * USDT;
    uint256 public constant MIN_MIN_ORDER = 1 * USDT;
    uint256 public constant MAX_MIN_ORDER = 100 * USDT;
    uint256 public constant MAX_ORDER_CAP = 100_000 * USDT;
    uint256 public constant MAX_TOTAL_LOCKED_CAP = 1_000_000 * USDT;
    uint256 public constant MAX_BASE_GAS = 1 * USDT;
    uint256 public constant MIN_DISPUTE_MUL = 5000;
    uint256 public constant MAX_DISPUTE_MUL = 50_000;
    uint256 public constant MIN_RESOLVE_TIMEOUT = 14 days;
    uint256 public constant MAX_RESOLVE_TIMEOUT = 60 days;
    uint256 public constant MIN_PROPOSAL_WINDOW = 1 days;
    uint256 public constant MAX_PROPOSAL_WINDOW = 7 days;
    uint256 public constant MAX_CAMPAIGN = 90 days;
    uint256 public constant MAX_GAS_SUBSIDY = 10_000;
    uint256 public constant DELAY = 72 hours;

    // Queue kinds for `queuedEta` keys.
    uint8 internal constant K_CAMPAIGN = 1;
    uint8 internal constant K_TOKEN = 2;
    uint8 internal constant K_SIGNER = 3;
    uint8 internal constant K_MARKET = 4;

    Params internal _params;
    Params internal _pending;
    uint64 public pendingEta; // 0 = no params queued
    bool public paused;

    Campaign internal _campaign;
    uint32 public subsidyUsed;

    /// @notice Funds currently at risk across all markets: open-order holdings + seller bonds (plan: loss is bounded
    ///         on-chain instead of by a paid audit). New orders and bond deposits revert past `totalLockedCap`.
    uint256 public totalLocked;

    address public auctionSigner; // forum-sponsor key that signs auction settlement proofs; 0 = auctions off
    mapping(address => bool) public tokenAllowed;
    mapping(address => bool) public isMarket; // may call `checkout` and lock / release seller bonds in Escrow
    mapping(bytes32 => uint64) public queuedEta;

    event ParamsQueued(Params params, uint64 eta);
    event ParamsApplied(Params params);
    event ParamsCancelled();
    event PausedSet(bool paused);
    event CampaignSet(Campaign campaign);
    event Queued(bytes32 indexed key, uint8 indexed kind, bytes data, uint64 eta);
    event QueueCancelled(bytes32 indexed key);
    event TokenSet(address indexed token, bool allowed);
    event AuctionSignerSet(address indexed signer);
    event MarketAdded(address indexed market);

    error BadParams();
    error BadState();
    error TooEarly();
    error NotMarket();
    error IsPaused();
    error TokenNotAllowed();
    error TotalLockedCap();

    /// @param markets_ Markets known at deploy time (the deploy script predicts Escrow's address).
    constructor(
        address owner_,
        Params memory params_,
        address token_,
        address auctionSigner_,
        address[] memory markets_
    ) Ownable(owner_) {
        _checkParams(params_);
        _params = params_;
        emit ParamsApplied(params_);
        _allowToken(token_);
        auctionSigner = auctionSigner_;
        emit AuctionSignerSet(auctionSigner_);
        for (uint256 i; i < markets_.length; ++i) {
            isMarket[markets_[i]] = true;
            emit MarketAdded(markets_[i]);
        }
    }

    // ───────────────────────────── Markets ─────────────────────────────

    /// @notice Snapshot for one new order; consumes a gas-subsidy slot when a subsidized campaign applies.
    ///         Reverts when paused or the token is not whitelisted.
    function checkout(address token, uint8 scope) external returns (Quote memory q) {
        if (!isMarket[msg.sender]) revert NotMarket();
        if (paused) revert IsPaused();
        if (!tokenAllowed[token]) revert TokenNotAllowed();
        q = _quote(scope);
        if (q.campaign && subsidyUsed < _campaign.gasSubsidyCap) {
            ++subsidyUsed;
            q.baseGas = 0;
        }
    }

    /// @notice Markets report funds entering custody. `enforceCap` is false for flows that must never be blocked
    ///         (e.g. a dispute fee pulled into an existing order).
    function lockFunds(uint256 amount, bool enforceCap) external {
        if (!isMarket[msg.sender]) revert NotMarket();
        uint256 total = totalLocked + amount;
        if (enforceCap && total > _params.totalLockedCap) revert TotalLockedCap();
        totalLocked = total;
    }

    /// @notice Markets report funds leaving custody (payouts, bond withdrawals).
    function unlockFunds(uint256 amount) external {
        if (!isMarket[msg.sender]) revert NotMarket();
        totalLocked -= amount;
    }

    /// @notice What `checkout` would return now, without consuming a subsidy slot (for the UI).
    function preview(uint8 scope) external view returns (Quote memory q) {
        q = _quote(scope);
        if (q.campaign && subsidyUsed < _campaign.gasSubsidyCap) q.baseGas = 0;
    }

    // ───────────────────────────── Params ─────────────────────────────

    /// @notice Loosening changes apply now; anything that raises charges or tightens rules waits 72 hours.
    ///         One pending slot: queueing again replaces it.
    function setParams(Params calldata p) external onlyOwner {
        _checkParams(p);
        if (_isLoosening(p, _params)) {
            _params = p;
            emit ParamsApplied(p);
        } else {
            _pending = p;
            pendingEta = uint64(block.timestamp + DELAY);
            emit ParamsQueued(p, pendingEta);
        }
    }

    function applyParams() external {
        // slither-disable-next-line incorrect-equality (0 is the "nothing queued" sentinel, not a balance)
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

    /// @notice Pausing only blocks new orders in every market; existing orders keep every exit.
    function setPaused(bool paused_) external onlyOwner {
        paused = paused_;
        emit PausedSet(paused_);
    }

    // ───────────────────────────── Campaign ─────────────────────────────

    /// @notice Schedule, replace or end (`end == 0`) the fee campaign. Applies at once unless a campaign is running
    ///         and the change would make it less generous (ending early = a price rise), which waits 72 hours.
    function setCampaign(Campaign calldata c) external onlyOwner {
        _checkCampaign(c);
        Campaign memory cur = _campaign;
        if (!_running(cur)) {
            _setCampaign(c, true);
        } else if (_atLeastAsGenerous(c, cur)) {
            _setCampaign(c, false); // extending a running campaign keeps the subsidy slots already used
        } else {
            _queue(K_CAMPAIGN, abi.encode(cur, c));
        }
    }

    /// @notice Anyone applies a queued campaign change, provided the campaign has not changed since it was queued.
    function applyCampaign(Campaign calldata c) external {
        _dequeue(K_CAMPAIGN, abi.encode(_campaign, c));
        _setCampaign(c, true);
    }

    // ───────────────────────────── Tokens, signer, markets ─────────────────────────────

    /// @notice Whitelist a new 6-decimal token after 72 hours (plan §1: e.g. native USDC later).
    function queueToken(address token) external onlyOwner {
        _checkToken(token);
        _queue(K_TOKEN, abi.encode(token));
    }

    function enableToken(address token) external {
        _dequeue(K_TOKEN, abi.encode(token));
        _allowToken(token);
    }

    /// @notice Immediate: only stops new orders and bond deposits in this token; existing orders are unaffected.
    function disableToken(address token) external onlyOwner {
        tokenAllowed[token] = false;
        emit TokenSet(token, false);
    }

    /// @dev Keyed on the current signer, so a stale queued change becomes void once the signer changes.
    function queueAuctionSigner(address signer) external onlyOwner {
        _queue(K_SIGNER, abi.encode(auctionSigner, signer));
    }

    function applyAuctionSigner(address signer) external {
        _dequeue(K_SIGNER, abi.encode(auctionSigner, signer));
        auctionSigner = signer;
        emit AuctionSignerSet(signer);
    }

    /// @notice Add a market (e.g. StreamEscrow) after 72 hours. Markets can lock and release seller bonds, so there
    ///         is no immediate path and no removal (a market must always be able to release what it locked).
    function queueMarket(address market) external onlyOwner {
        _queue(K_MARKET, abi.encode(market));
    }

    function addMarket(address market) external {
        _dequeue(K_MARKET, abi.encode(market));
        isMarket[market] = true;
        emit MarketAdded(market);
    }

    function cancelQueued(bytes32 key) external onlyOwner {
        if (queuedEta[key] == 0) revert BadState();
        delete queuedEta[key];
        emit QueueCancelled(key);
    }

    // ───────────────────────────── Views ─────────────────────────────

    function params() external view returns (Params memory) {
        return _params;
    }

    function pendingParams() external view returns (Params memory) {
        return _pending;
    }

    function campaign() external view returns (Campaign memory) {
        return _campaign;
    }

    function queueKey(uint8 kind, bytes memory data) public pure returns (bytes32) {
        return keccak256(abi.encode(kind, data));
    }

    // ───────────────────────────── Internal ─────────────────────────────

    function _quote(uint8 scope) internal view returns (Quote memory q) {
        Params memory p = _params;
        Campaign memory c = _campaign;
        q.campaign = _running(c) && (c.scope & scope) != 0;
        q.feeBps = q.campaign && c.rateBps < p.feeBps ? c.rateBps : p.feeBps;
        q.minFee = q.campaign ? 0 : p.minFee;
        q.baseGas = p.baseGas;
        q.disputeMinFee = p.minFee;
        q.minOrder = p.minOrder;
        q.orderCap = p.orderCap;
        q.disputeMulBps = p.disputeMulBps;
        q.resolveTimeout = p.resolveTimeout;
        q.proposalWindow = p.proposalWindow;
    }

    function _queue(uint8 kind, bytes memory data) internal {
        bytes32 key = queueKey(kind, data);
        uint64 eta = uint64(block.timestamp + DELAY);
        queuedEta[key] = eta;
        emit Queued(key, kind, data, eta);
    }

    function _dequeue(uint8 kind, bytes memory data) internal {
        bytes32 key = queueKey(kind, data);
        uint64 eta = queuedEta[key];
        if (eta == 0) revert BadState();
        if (block.timestamp < eta) revert TooEarly();
        delete queuedEta[key];
    }

    function _allowToken(address token) internal {
        _checkToken(token);
        tokenAllowed[token] = true;
        emit TokenSet(token, true);
    }

    /// @dev All amounts (fees, caps, gas) are denominated in 6-decimal units, so only 6-decimal tokens qualify.
    function _checkToken(address token) internal view {
        if (token == address(0) || IERC20Metadata(token).decimals() != 6) revert BadParams();
    }

    function _setCampaign(Campaign memory c, bool resetSubsidy) internal {
        _campaign = c;
        if (resetSubsidy) subsidyUsed = 0;
        emit CampaignSet(c);
    }

    function _running(Campaign memory c) internal view returns (bool) {
        return c.start <= block.timestamp && block.timestamp < c.end;
    }

    function _atLeastAsGenerous(Campaign memory n, Campaign memory c) internal view returns (bool) {
        return n.start <= block.timestamp && n.end >= c.end && n.rateBps <= c.rateBps && (n.scope & c.scope) == c.scope
            && n.gasSubsidyCap >= c.gasSubsidyCap;
    }

    function _checkCampaign(Campaign calldata c) internal view {
        if (c.end == 0) {
            if (c.start != 0 || c.rateBps != 0 || c.scope != 0 || c.gasSubsidyCap != 0) revert BadParams();
            return;
        }
        if (
            c.end <= c.start || c.end - c.start > MAX_CAMPAIGN || c.end <= block.timestamp || c.rateBps > MAX_FEE_BPS
                || c.scope == 0 || c.scope > (SCOPE_ESCROW | SCOPE_AUCTION) || c.gasSubsidyCap > MAX_GAS_SUBSIDY
        ) revert BadParams();
    }

    function _checkParams(Params memory p) internal pure {
        if (
            p.feeBps > MAX_FEE_BPS || p.minFee > MAX_MIN_FEE || p.minOrder < MIN_MIN_ORDER || p.minOrder > MAX_MIN_ORDER
                || p.orderCap < p.minOrder || p.orderCap > MAX_ORDER_CAP || p.totalLockedCap < p.orderCap
                || p.totalLockedCap > MAX_TOTAL_LOCKED_CAP || p.baseGas > MAX_BASE_GAS
                || p.disputeMulBps < MIN_DISPUTE_MUL || p.disputeMulBps > MAX_DISPUTE_MUL
                || p.resolveTimeout < MIN_RESOLVE_TIMEOUT || p.resolveTimeout > MAX_RESOLVE_TIMEOUT
                || p.proposalWindow < MIN_PROPOSAL_WINDOW || p.proposalWindow > MAX_PROPOSAL_WINDOW
        ) revert BadParams();
    }

    /// @dev True when no field raises a charge or tightens a rule. Timeouts and windows count as rules: any change
    ///      to them is queued. `orderCap` and `totalLockedCap` are exempt in both directions: raising is looser, and
    ///      lowering only blocks new orders (a risk brake that must act at once), never touching open ones.
    function _isLoosening(Params memory n, Params memory c) internal pure returns (bool) {
        return n.feeBps <= c.feeBps && n.minFee <= c.minFee && n.minOrder <= c.minOrder && n.baseGas <= c.baseGas
            && n.disputeMulBps <= c.disputeMulBps && n.resolveTimeout == c.resolveTimeout
            && n.proposalWindow == c.proposalWindow;
    }
}
