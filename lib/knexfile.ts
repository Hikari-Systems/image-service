import fs from 'fs';
import path from 'path';
import knex, { Knex } from 'knex';
import snakeCase from 'lodash.snakecase';
import camelCase from 'lodash.camelcase';
import { config, logging } from '@hikari-systems/hs.utils';

const log = logging('server');

const configBoolean = (key: string, defaultValue = false): boolean =>
  (config.get(key) || String(defaultValue)).trim() === 'true';

const configInteger = (key: string, defaultValue: number): number =>
  config.get(key) ? parseInt(config.get(key), 10) : defaultValue;

const configString = (key: string, defaultValue = ''): string =>
  (config.get(key) || defaultValue).trim();

const getSslConfig = (): Knex.PgConnectionConfig['ssl'] => {
  if (!configBoolean('db:ssl:enabled')) {
    return false;
  }

  const rejectUnauthorized = configBoolean('db:ssl:verify');

  const caCertPath = configString('db:ssl:caCertFile');
  if (caCertPath === '') {
    return {
      rejectUnauthorized,
    };
  }
  return {
    rejectUnauthorized,
    ca: fs.readFileSync(caCertPath),
  };
};

export const wrapIdentifier: NonNullable<Knex.Config['wrapIdentifier']> = (
  value,
  wrap,
) => wrap(value === '*' ? '*' : snakeCase(value));

export const postProcessResponse = ((): NonNullable<
  Knex.Config['postProcessResponse']
> => {
  const toCamelCase = (x: object) =>
    Object.keys(x).reduce((acc, key) => {
      // @ts-expect-error old syntax for expansion
      acc[camelCase(key)] = x[key];
      return acc;
    }, {});

  return (result) => {
    if (Array.isArray(result)) {
      return result.map(toCamelCase);
    }

    if (typeof result === 'object' && result !== null) {
      return toCamelCase(result);
    }

    return result;
  };
})();

const knexConfig = {
  main: {
    client: 'pg',
    debug: configBoolean('db:debug'),
    connection: {
      host: config.get('db:host'),
      port: config.get('db:port'),
      database: config.get('db:database'),
      user: config.get('db:username'),
      password: config.get('db:password'),
      ssl: getSslConfig(),
    } as Knex.PgConnectionConfig,
    pool: {
      min: configInteger('db:minpool', 0),
      max: configInteger('db:maxpool', 10),
    } as Knex.PoolConfig,
    migrations: {
      directory: path.join(__dirname, '/../migrations'),
      tableName: 'knex_migrations',
      loadExtensions: ['.js'],
    } as Knex.MigratorConfig,
    wrapIdentifier,
    postProcessResponse,
  } as Knex.Config,
};

export async function runKnexMigrations() {
  const db: Knex = knex(knexConfig.main);
  try {
    const [batchNo, logMigrations] = await db.migrate.latest();
    if (logMigrations.length > 0) {
      log.debug(`Batch ${batchNo} run: ${logMigrations.length} migrations`);
      logMigrations.forEach((m: string) => log.debug(`> ${m}`));
    } else {
      log.debug('No migrations applied');
    }
    log.debug('Database migrations completed');
  } catch (err) {
    log.error('Migration failed', err);
    process.exit(1);
  } finally {
    await db.destroy();
  }
}

export default knexConfig;
