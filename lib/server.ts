import express from 'express';
import onDeath from 'death';
import { config, logging } from '@hikari-systems/hs.utils';
import routes from './index';

const log = logging('server');

const app = express();
app.use(routes);

const port = parseInt(config.get('server:port') || '3000', 10);
const server = app.listen(port, async () => {
  log.debug(
    `Image-service listening on port ${port}: go to http://localhost:${port}/`,
  );
});

onDeath(() => {
  server.close();
});
