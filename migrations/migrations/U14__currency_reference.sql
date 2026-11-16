-- Phase 0 hardening: currency reference data.
--
-- Amounts are integer minor units, so the number of decimal places is part of
-- the meaning of a value: 1000 is ten units in a 2-decimal currency and one
-- thousand units in a 0-decimal currency. The app assumed 2 everywhere.
--
-- This table is the reference for anything SQL-side. The Rust side carries the
-- same list for formatting (it cannot query per call); an integration test keeps
-- the two in lockstep and pins both to the ISO-4217 values.
CREATE TABLE IF NOT EXISTS currency (
    code        CHAR(3)  NOT NULL PRIMARY KEY,
    minor_unit  SMALLINT NOT NULL CHECK (minor_unit >= 0 AND minor_unit <= 4),
    name        TEXT     NOT NULL
);

INSERT INTO currency (code, minor_unit, name) VALUES
    ('USD', 2, 'US dollar'),
    ('EUR', 2, 'Euro'),
    ('GBP', 2, 'Pound sterling'),
    ('CHF', 2, 'Swiss franc'),
    ('CAD', 2, 'Canadian dollar'),
    ('AUD', 2, 'Australian dollar'),
    ('NZD', 2, 'New Zealand dollar'),
    ('SEK', 2, 'Swedish krona'),
    ('NOK', 2, 'Norwegian krone'),
    ('DKK', 2, 'Danish krone'),
    ('PLN', 2, 'Polish zloty'),
    ('MXN', 2, 'Mexican peso'),
    ('BRL', 2, 'Brazilian real'),
    ('CNY', 2, 'Yuan renminbi'),
    ('INR', 2, 'Indian rupee'),
    ('TRY', 2, 'Turkish lira'),
    ('ZAR', 2, 'South African rand'),
    ('JPY', 0, 'Japanese yen'),
    ('KRW', 0, 'South Korean won'),
    ('CLP', 0, 'Chilean peso'),
    ('ISK', 0, 'Icelandic krona'),
    ('VND', 0, 'Vietnamese dong'),
    ('KWD', 3, 'Kuwaiti dinar'),
    ('BHD', 3, 'Bahraini dinar'),
    ('OMR', 3, 'Omani rial'),
    ('TND', 3, 'Tunisian dinar')
ON CONFLICT (code) DO NOTHING;
