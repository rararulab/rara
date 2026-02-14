-- Add PDF storage fields to resume table
ALTER TABLE resume ADD COLUMN pdf_object_key TEXT;
ALTER TABLE resume ADD COLUMN pdf_file_size BIGINT;
