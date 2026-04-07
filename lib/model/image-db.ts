import { Knex } from 'knex';
import { v4 } from 'uuid';
import { ImageType } from '../types/types';

// Use explicit snake_case keys so wrapIdentifier (lodash snakeCase) is a no-op.
// Spreading ImageType directly causes lodash to mangle names like
// downloadedS3Path → downloaded_s_3_path. We also must JSON.stringify jsonb arrays.
const toRow = (image: ImageType) => ({
  id: image.id,
  category: image.category,
  source_url: image.sourceUrl ?? null,
  downloaded_s3_path: image.downloadedS3Path ?? null,
  original_s3_path: image.originalS3Path ?? null,
  resized_files: image.resizedFiles ? JSON.stringify(image.resizedFiles) : null,
  avoid_resize_until: image.avoidResizeUntil ?? null,
  created_at: image.createdAt ?? null,
});

const get =
  (db: Knex) =>
  (id: string): Promise<ImageType | null> =>
    db
      .select()
      .from('image')
      .where('id', id)
      .then((r) => (r.length ? r[0] : null));

const upsert = (db: Knex) => (image: ImageType): Promise<ImageType> => {
  const id = image?.id || v4();
  const insertRow = { ...toRow({ ...image, id }), created_at: new Date() };
  const updateRow = { ...toRow({ ...image, id }), updated_at: new Date() };
  return db
    .insert(insertRow)
    .into('image')
    .onConflict('id')
    .merge(updateRow)
    .returning('*')
    .then((r) => r[0]);
};

export default (db: Knex) => ({
  get: get(db),
  upsert: upsert(db),
});
