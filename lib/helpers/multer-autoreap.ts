/* eslint-disable */
const fs = require('fs');
const finished = require('on-finished');
const debug = require('debug')('multer-autoreap:middleware');

// auto remove any uploaded files on response end
// to persist uploaded files, simply move them to a permanent location,
// or delete the req.files[key] before the response end.
export default (options: any) =>
  function (req: any, res: any, next: any) {
    const processFile = function processFile(err: Error, file: any) {
      if (err && options && !options.reapOnError) {
        debug(
          'skipped auto removal of %s - please manually deprecate.',
          file.path,
        );
        return;
      }
      fs.stat(file.path, (err: Error, stats: any) => {
        if (!err && stats.isFile()) {
          fs.unlink(file.path, (err: Error) => {
            if (err) return console.warn(err);
            debug('removed %s', file.path);
            res.emit('autoreap', file);
          });
        }
      });
    };

    const reapFiles = function reapFiles(err: any) {
      let done: any = [];
      let queue: any = [];

      if (req.file) {
        queue = queue.concat(req.file);
        debug('queued %s', req.file);
      }

      if (req.files) {
        if (Array.isArray(req.files)) {
          queue = queue.concat(req.files);
          debug('queued %O', req.files);
        } else {
          Object.entries(req.files).forEach(([key, files]) => {
            queue = queue.concat(files as any);
            debug('queued %s, %O', key, files);
          });
        }
      }

      queue.forEach((file: any) => {
        if (!done.includes(file)) {
          processFile(err, file);
          done.push(file);
        }
      });
    };

    finished(res, reapFiles);
    next();
  };
