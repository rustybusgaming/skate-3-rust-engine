// Precompressed copies of the browser package's files for skate-web-server, which sends the
// smallest one a browser accepts (docs/web-port/server.md): <file>.gz and <file>.br beside each.
//
//   node tools/web-port/compress.mjs <brotli quality 0-11> <file>...
//
// Brotli's largest window (16 MB) and quality 11 make the engine about 40 % smaller than gzip -9,
// at about three minutes of compression; quality 9 takes seconds.
import { readFileSync, writeFileSync } from 'node:fs';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const [quality, ...files] = process.argv.slice(2);
for (const file of files) {
  const bytes = readFileSync(file);
  writeFileSync(`${file}.gz`, gzipSync(bytes, { level: 9 }));
  writeFileSync(`${file}.br`, brotliCompressSync(bytes, {
    params: {
      [constants.BROTLI_PARAM_QUALITY]: Number(quality),
      [constants.BROTLI_PARAM_LGWIN]: constants.BROTLI_MAX_WINDOW_BITS,
      [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length,
    },
  }));
}
