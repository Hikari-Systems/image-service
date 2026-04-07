import { Knex } from 'knex';

export async function up(knex: Knex): Promise<void> {
  await knex.schema.createTable('image', (t) => {
    t.uuid('id').primary().notNullable();
    t.string('category', 255);
    t.text('source_url');
    t.text('downloaded_s3_path');
    t.text('original_s3_path');
    t.jsonb('resized_files');
    t.timestamp('avoid_resize_until');
    t.timestamp('created_at');
    t.timestamp('updated_at');
  });
}

export async function down(knex: Knex): Promise<void> {
  await knex.schema.dropTable('image');
}
