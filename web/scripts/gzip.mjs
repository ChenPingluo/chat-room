import { gzipSync } from "node:zlib";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

async function walk(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...(await walk(path)));
    else files.push(path);
  }
  return files;
}

const files = await walk(new URL("../dist", import.meta.url).pathname);
for (const file of files) {
  if (file.endsWith(".gz") || file.endsWith(".png") || file.endsWith(".jpg")) continue;
  const data = await readFile(file);
  if (data.length < 256) continue;
  await writeFile(`${file}.gz`, gzipSync(data, { level: 9 }));
}
