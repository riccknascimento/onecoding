-- Limite de tentativas de login também por origem (IP). Falhas antigas ficam com origem vazia e só
-- contam para o bloqueio por conta, como antes.
ALTER TABLE login_failures ADD COLUMN origin text NOT NULL DEFAULT '' CHECK (length(origin) <= 64);
CREATE INDEX login_failures_origin_time ON login_failures(origin, failed_at);
