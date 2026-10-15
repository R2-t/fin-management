# Currency figures are compared within one wallet, never across wallets

We decided not to consolidate amounts across wallets holding different currencies, so `purchase_fact`
stores only the purchase currency amount and the wallet currency amount plus the rate between them.
An earlier plan proposed a third amount converted into a user-level reporting currency; it was dropped
because the stored rate maps purchase currency to wallet currency only, so cross-wallet consolidation
would require a second rate we neither store nor can reconstruct for past purchases. Consequence:
users cannot see a single net-worth figure spanning currencies, and forecasts are per-wallet.
