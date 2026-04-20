CREATE TABLE IF NOT EXISTS wallet (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    name VARCHAR NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ,
    UNIQUE(user_id, name)
);

CREATE TABLE IF NOT EXISTS category (
    id INTEGER PRIMARY KEY,
    name VARCHAR NOT NULL,
    description TEXT,
    user_id UUID,
    is_system BOOL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT category_user_system_check
        CHECK ((is_system = TRUE AND user_id IS NULL) OR (is_system = FALSE AND user_id IS NOT NULL))
    );

CREATE TABLE IF NOT EXISTS purchase (
    id UUID PRIMARY KEY NOT NULL,
    user_id UUID NOT NULL,
    wallet_id UUID NOT NULL REFERENCES wallet(id),
    store VARCHAR,
    category_id INTEGER NOT NULL REFERENCES category (id) ON DELETE CASCADE,
    description TEXT,
    notes TEXT,
    version INT DEFAULT 1,
    purchase_date TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_purchase_user_category ON purchase(user_id, category_id);

CREATE INDEX IF NOT EXISTS idx_purchase_user_date ON purchase(user_id, purchase_date DESC);

CREATE INDEX IF NOT EXISTS idx_purchase_wallet ON purchase(wallet_id, purchase_date DESC);

CREATE TABLE IF NOT EXISTS item (
    item_sequence INTEGER NOT NULL,
    name VARCHAR NOT NULL,
    unit_price NUMERIC NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    purchase_id UUID NOT NULL REFERENCES purchase(id),
    item_type VARCHAR,
    PRIMARY KEY (purchase_id, item_sequence)
);

CREATE OR REPLACE VIEW purchase_items AS
    SELECT p.id as purchase_id,
           p.store as store,
           c.name as category,
           p.purchase_date as purchase_date,
           p.user_id as user_id,
           p.wallet_id as wallet_id,
           p.description as description,
           p.notes as notes,
           p.created_at as created_at,
           p.updated_at as updated_at,
           sum(i.quantity * i.unit_price) as total,
           json_agg(
                json_build_object(
                    'sequence', i.item_sequence,
                    'name', i.name,
                    'unit_price', i.unit_price,
                    'quantity', i.quantity,
                    'item_type', i.item_type
                )
           ) as items
    FROM purchase p
        JOIN item i on p.id = i.purchase_id
        JOIN category c on p.category_id = c.id
    WHERE p.deleted_at IS NULL
    GROUP BY p.id, c.name;

CREATE TABLE IF NOT EXISTS purchase_history (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    purchase_id UUID NOT NULL REFERENCES purchase(id),
    version INT NOT NULL,
    changes JSONB NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_purchase_history_id ON purchase_history(purchase_id, version DESC)
