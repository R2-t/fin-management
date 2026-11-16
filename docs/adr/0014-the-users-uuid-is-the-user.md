# The User's UUID is the User

Every table in this service records who a row belongs to with `user_id UUID`, and
until now that UUID pointed at nothing. There was no `users` table: identity lived
outside the database, and a `user_id` was a name the API trusted. Adding login means
adding a table that holds a password hash, and the obvious design is a new
`users` table with its own primary key, with every `user_id` remapped to it.

We are not doing that. The `users` table's `id` **is** the UUID that is already
everywhere.

The alternative — a second identity, with the opaque UUID pointing at it — buys a
cleaner-looking schema and costs a migration that rewrites tenancy across every table,
every index that includes `user_id`, and every one of 184 integration tests. The
containment it offers is not real: the blast radius of a wrong `user_id` is decided by
which value the code puts in the `WHERE` clause, not by whether a foreign key can be
checked later. What does change is the amount of code that has to be correct during the
change, and that is the quantity to minimise in an authentication ticket.

One consequence is deliberate: `user_profile` does **not** gain a foreign key to
`users` in this phase. The integration suite fabricates a fresh random `user_id` per
test, at 97 call sites; a foreign key would force an async seeding path through every
test file, so that every test would have to create a user before it could create a
purchase. That is a large unrelated refactor inside an auth change, for integrity the
write paths already have: every production write resolves its `user_id` from a session
or from a signed claim, both of which derive from a real row.

The gap this leaves is worth naming plainly: a purchase can exist whose `user_id` has
no `users` row, and nothing in the database forbids it. That is the same gap that
exists today, and closing it is a data-integrity task with its own ticket, where the
test fixtures can be changed properly rather than in passing.
