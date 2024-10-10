// import { config, logging } from '@hikari-systems/hs.utils';
import { timingMiddleware } from '@hikari-systems/hs.utils';
import express from 'express';
import path from 'path';
import imageRoutes from './route/image';

// const log = logging('index');

const app = express.Router();

// root healthcheck is deprecated
app.get('/healthcheck', (_req, res) => res.status(200).send('OK'));

app.use(timingMiddleware);
app.use(imageRoutes);

app.use('/test', express.static(path.join(__dirname, '../static')));

export default app;
