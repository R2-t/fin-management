**Agent Role:** Backend System Architect & Developer Agent
**Task Scope:** Enhance Purchase Currency Handling within User Wallets Architecture
**Priority:** High (Critical Business Logic)

### 1. Context & Current State
You are working on an extension of a user wallet service architecture with the following existing constraints:
*   **Structure:** One `User` owns multiple `Wallet`.
*   **Purchase Model:** A Purchase does **not** store a static total value field in its database table. Instead, the transactional view calculates the total
dynamically via a View Table or Materialized Logic by aggregating `(Item_Quantity * Item_Unit_Price)`.
*   **Currency Context:** The `Wallet` implies a specific holding currency for that user's balance tracking (e.g., Wallet USD). However, purchases can
originate from various external merchants/currencies.

### 2. Functional Requirements
Implement the following logic to handle purchasing across different currencies:

1.  **Source Currency Attribute**: Every `Item` or Line Item within a Purchase must store its original transaction currency code and unit price (e.g., EUR for
an item purchased in Europe). Do not remove existing fields, but ensure this data persists. Add one field in the purchase table called currency, it will indicate what currency the items of the purchase is. Check it against the wallet currency (if wallet doesn't have a currency field, add it as well) and if they don't match at the time of creating the purchase, add a exchange rate within the purchase (I'm still unsure about it, but every purchase might have a different one so that's why I think denormalizing it into the purchase table might be a good idea, however it will mean that the purchase table will have many responsabilities) 
2.  **Conversion Logic Implementation**: Before calculating the view-based total for deduction from the Wallet:
    *   **Primary Method:** Call a secured External Currency API to fetch real-time exchange rates (Target Rate) based on the `Purchase_Date`. Convert all
Line Item values into the specific `Wallet_Currency` before aggregation.
    *   **Secondary/Manual Override**: If the User has knowledge of exact value/rate (e.g., fixed transfer, crypto volatility), allow a manual input field to
override the API rate or final total in the Purchase metadata JSON payload without violating schema constraints.
3.  **View Calculation Integrity**: Do not introduce redundant `TOTAL_AMOUNT` columns if possible; maintain the rule that totals are aggregated from line
items. However, you may need to add a `Rate_At_Timestamp` field to support accurate historical ledgering of this conversion logic within the view query or
application-layer calculation engine.
4.  **Ledger Deduction**: Ensure Wallet deduction happens in the *Wallet's* native currency after all applicable exchange rates have been applied during
purchase creation/update, ensuring no over/under-deduction issues occur due to API latency.

### 3. Implementation Instructions
Please provide a solution covering the following areas:

1.  **Data Schema Analysis**: Propose minimal schema changes required (if any) or metadata adjustments needed to handle source currency codes without
violating dynamic calculation rules. *Crucial:* Ensure existing View logic for totals isn't broken when exchange rates are applied during insert/update
operations.
2.  **Logic Flow Diagram/Code Strategy**: Define the exact order of operations:
    *   Input Validation (Currency Code) $\rightarrow$ API Rate Fetch vs User Override Flag $\rightarrow$ Application Layer Conversion Logic $\rightarrow$
View Recalculation trigger or real-time aggregation logic in application layer (depending on your system preference).
3.  **Error Handling Strategy**: How do you handle cases where:
    *   The External Currency API fails?
    *   No valid rate is returned for a currency pair?
    *   User inputs an unrealistic manual conversion that deviates from market value significantly? (Define validation thresholds or logging).
4.  **API Contract Specification**: If this involves external calls, outline the expected payload structure for sending purchase details to your own API
endpoint where these rates are determined before processing against the wallet balance.

### 4. Constraints & Notes
*   Do not store calculated currency totals in a fixed column if it conflicts with our existing requirement that *purchases don't have total value stored, but
instead is calculated*. Ensure any conversion logic happens at calculation time or through an immutable rate lookup joined during view generation.
*   Preserve User Privacy: Do not log sensitive raw payment data unnecessarily; only currency codes and transaction IDs should be logged for audit purposes
regarding exchange rates.
*   Consistency Check: The sum of converted line items must exactly match the amount deducted from the specific Wallet's balance ledger after rounding logic
is applied (define precision/routing rules).

### 5. Expected Deliverable Format
1.  Updated Entity-Relationship diagram snippet or ERD text showing currency relationships.
2.  Pseudo-code/Logic Flow for calculating `Effective_Wallet_Deduction`.
3.  Proposed SQL modification to the View Table if required to include rate metadata, OR instructions on how to handle rates in the Application Service Layer
without modifying Views.
