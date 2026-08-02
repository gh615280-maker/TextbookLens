ALTER TABLE app_settings ADD COLUMN font_scale REAL NOT NULL DEFAULT 1.0 CHECK (font_scale >= 0.75 AND font_scale <= 2.0);
ALTER TABLE app_settings ADD COLUMN line_height REAL NOT NULL DEFAULT 1.6 CHECK (line_height >= 1.2 AND line_height <= 2.4);
ALTER TABLE app_settings ADD COLUMN reader_width REAL NOT NULL DEFAULT 72 CHECK (reader_width >= 40 AND reader_width <= 120);
ALTER TABLE app_settings ADD COLUMN pdf_zoom REAL NOT NULL DEFAULT 1.0 CHECK (pdf_zoom >= 0.5 AND pdf_zoom <= 3.0);
