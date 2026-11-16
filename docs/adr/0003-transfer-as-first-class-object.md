# Transfers are a first-class object that generates ledger entries

A transfer between two of the user's own wallets is stored as its own record (amount, source,
destination, date, status) and the debit/credit ledger entries are derived from it, rather than being
two rows tied together by a shared group key. We chose this because correcting or deleting a mistaken
transfer is a normal action in a record-your-spending app, and a single owned object makes edit and
delete coherent instead of requiring both legs to be found and mutated consistently. Ledger legs must
never be edited directly. Income split across several wallets uses the same shape: one intentional
object generating many entries.
