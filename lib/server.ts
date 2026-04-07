import express from 'express';
import onDeath from 'death';
import { config, logging } from '@hikari-systems/hs.utils';
import routes from './index';
import { runKnexMigrations } from './knexfile';

const log = logging('server');

const startServer = () => {
  const app = express();
  app.use(routes);
  const port = parseInt(config.get('server:port') || '3000', 10);
  const server = app.listen(port, () => {
    log.debug(
      `Image-service listening on port ${port}: go to http://localhost:${port}/`,
    );
  });
  onDeath(() => {
    server.close();
  });
};

const storage = (config.get('imageMetadata:storage') || 'file').trim();
if (storage === 'db') {
  runKnexMigrations()
    .then(startServer)
    .catch((err: Error) => {
      log.error('Migration failed, refusing to start', err);
      process.exit(1);
    });
} else {
  startServer();
}
