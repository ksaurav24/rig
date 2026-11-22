-- Network throughput in bytes per second, averaged over the sample. NULL for
-- the first sample of a scope and after a counter reset (container restart).
ALTER TABLE metrics ADD COLUMN net_rx REAL;
ALTER TABLE metrics ADD COLUMN net_tx REAL;
