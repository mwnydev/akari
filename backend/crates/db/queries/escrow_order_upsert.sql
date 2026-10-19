-- 写入折叠后的订单。只有内容确实变化时才更新并返回 id（调用方据此决定是否写发件箱）。
-- 金额以十进制文本传入再转 numeric（uint256 不经过浮点或 i64）。
INSERT INTO escrow_orders AS o (
  chain_id, contract, chain_order_id, buyer, seller, buyer_id, seller_id, token, amount,
  buyer_fee, seller_fee, buyer_gas, seller_gas, dispute_fee, bond,
  instant, offer_hash, listing_hash, auction_id, state, ship_deadline, confirm_deadline,
  proposer, proposal_to_buyer, proposal_expiry, disputed_by, dispute_from_bond,
  ruling_to_buyer_bps, ruling_bond_to_buyer_bps, settled_to_buyer, settled_to_seller, settled_to_fee,
  snapshot, level, created_block, updated_block
) VALUES (
  $1, $2, $3, $4, $5,
  (SELECT user_id FROM wallets WHERE chain_id = $1 AND address = $4),
  (SELECT user_id FROM wallets WHERE chain_id = $1 AND address = $5),
  $6, $7::text::numeric,
  $8::text::numeric, $9::text::numeric, $10::text::numeric, $11::text::numeric, $12::text::numeric, $13::text::numeric,
  $14, $15, $16, $17::text::numeric, $18, $19, $20,
  $21, $22::text::numeric, $23, $24, $25,
  $26, $27, $28::text::numeric, $29::text::numeric, $30::text::numeric,
  $31, $32::text::chain_level, $33, $34
)
ON CONFLICT (chain_id, contract, chain_order_id) DO UPDATE SET
  buyer = EXCLUDED.buyer, seller = EXCLUDED.seller, buyer_id = EXCLUDED.buyer_id, seller_id = EXCLUDED.seller_id,
  token = EXCLUDED.token, amount = EXCLUDED.amount,
  buyer_fee = EXCLUDED.buyer_fee, seller_fee = EXCLUDED.seller_fee, buyer_gas = EXCLUDED.buyer_gas,
  seller_gas = EXCLUDED.seller_gas, dispute_fee = EXCLUDED.dispute_fee, bond = EXCLUDED.bond,
  instant = EXCLUDED.instant, offer_hash = EXCLUDED.offer_hash, listing_hash = EXCLUDED.listing_hash,
  auction_id = EXCLUDED.auction_id, state = EXCLUDED.state,
  ship_deadline = EXCLUDED.ship_deadline, confirm_deadline = EXCLUDED.confirm_deadline,
  proposer = EXCLUDED.proposer, proposal_to_buyer = EXCLUDED.proposal_to_buyer,
  proposal_expiry = EXCLUDED.proposal_expiry, disputed_by = EXCLUDED.disputed_by,
  dispute_from_bond = EXCLUDED.dispute_from_bond, ruling_to_buyer_bps = EXCLUDED.ruling_to_buyer_bps,
  ruling_bond_to_buyer_bps = EXCLUDED.ruling_bond_to_buyer_bps, settled_to_buyer = EXCLUDED.settled_to_buyer,
  settled_to_seller = EXCLUDED.settled_to_seller, settled_to_fee = EXCLUDED.settled_to_fee,
  snapshot = EXCLUDED.snapshot, level = EXCLUDED.level,
  created_block = EXCLUDED.created_block, updated_block = EXCLUDED.updated_block, updated_at = now()
WHERE (o.buyer, o.seller, o.buyer_id, o.seller_id, o.token, o.amount, o.buyer_fee, o.seller_fee, o.buyer_gas,
       o.seller_gas, o.dispute_fee, o.bond, o.instant, o.offer_hash, o.listing_hash, o.auction_id, o.state,
       o.ship_deadline, o.confirm_deadline, o.proposer, o.proposal_to_buyer, o.proposal_expiry, o.disputed_by,
       o.dispute_from_bond, o.ruling_to_buyer_bps, o.ruling_bond_to_buyer_bps, o.settled_to_buyer,
       o.settled_to_seller, o.settled_to_fee, o.snapshot, o.level, o.created_block, o.updated_block)
  IS DISTINCT FROM
      (EXCLUDED.buyer, EXCLUDED.seller, EXCLUDED.buyer_id, EXCLUDED.seller_id, EXCLUDED.token, EXCLUDED.amount,
       EXCLUDED.buyer_fee, EXCLUDED.seller_fee, EXCLUDED.buyer_gas, EXCLUDED.seller_gas, EXCLUDED.dispute_fee,
       EXCLUDED.bond, EXCLUDED.instant, EXCLUDED.offer_hash, EXCLUDED.listing_hash, EXCLUDED.auction_id,
       EXCLUDED.state, EXCLUDED.ship_deadline, EXCLUDED.confirm_deadline, EXCLUDED.proposer,
       EXCLUDED.proposal_to_buyer, EXCLUDED.proposal_expiry, EXCLUDED.disputed_by, EXCLUDED.dispute_from_bond,
       EXCLUDED.ruling_to_buyer_bps, EXCLUDED.ruling_bond_to_buyer_bps, EXCLUDED.settled_to_buyer,
       EXCLUDED.settled_to_seller, EXCLUDED.settled_to_fee, EXCLUDED.snapshot, EXCLUDED.level,
       EXCLUDED.created_block, EXCLUDED.updated_block)
RETURNING id, buyer_id, seller_id
