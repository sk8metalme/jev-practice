import { readdir } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

async function javascriptFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(entries.map(entry => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? javascriptFiles(path) : entry.name.endsWith('.js') ? [path] : [];
  }));
  return nested.flat();
}

const files = [
  ...(await javascriptFiles('src')),
  ...(await javascriptFiles('scripts')),
];
const failures = [];

for (const file of files) {
  try {
    await execFileAsync(process.execPath, ['--check', file]);
  } catch (error) {
    failures.push({ file, message: error.stderr?.trim() ?? error.message });
  }
}

if (failures.length > 0) {
  for (const failure of failures) {
    console.error(`${failure.file}: ${failure.message}`);
  }
  process.exitCode = 1;
} else {
  console.log(`lint ok: ${files.length} JavaScript files`);
}
