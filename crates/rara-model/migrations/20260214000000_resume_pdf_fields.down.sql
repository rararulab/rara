-- Remove PDF storage fields from resume table
ALTER TABLE resume DROP COLUMN IF EXISTS pdf_object_key;
ALTER TABLE resume DROP COLUMN IF EXISTS pdf_file_size;
