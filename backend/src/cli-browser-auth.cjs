// Authentication only: load the installed official Gemini OAuth export, never its CLI main or a model.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const LIMIT = 1024 * 1024;

async function findAuthModule(bundleDir) {
  const entry = await fs.promises.readFile(path.join(bundleDir, 'gemini.js'), 'utf8');
  const imports = [...entry.matchAll(/(?:from\s*|import\s*)["']\.\/([^"']+\.js)["']/g)].map(match => match[1]);
  const files = [...new Set([...imports, ...(await fs.promises.readdir(bundleDir)).filter(name => name.endsWith('.js')).sort()])];
  for (const name of files) {
    if (path.basename(name) !== name) continue;
    const filename = path.join(bundleDir, name);
    if ((await fs.promises.stat(filename)).size > 32 * LIMIT) continue;
    const source = await fs.promises.readFile(filename, 'utf8');
    const exports = source.slice(source.lastIndexOf('export {'));
    if (/async function getOauthClient\(/.test(source) && /\bgetOauthClient\s*[,}]/.test(exports)) return filename;
  }
  throw new Error('Unsupported official Gemini OAuth exports');
}

function scopedPath(profile, filename) {
  const resolved = path.resolve(filename);
  // Rust canonicalize uses \\?\ Windows paths; compare both paths in the same namespace.
  const relative = path.relative(path.toNamespacedPath(path.resolve(profile)), path.toNamespacedPath(resolved));
  if (!relative || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) throw new Error('Invalid isolated credential path');
  return resolved;
}

function credentialPayload(credentials) {
  if (!credentials || !['access_token', 'refresh_token'].some(key => typeof credentials[key] === 'string' && credentials[key].trim())) throw new Error('Missing OAuth credentials');
  const payload = JSON.stringify(credentials);
  if (Buffer.byteLength(payload, 'utf8') > LIMIT) throw new Error('OAuth credentials exceed limit');
  return payload;
}

async function authorize(bundleDir, profile) {
  profile = await fs.promises.realpath(profile);
  if (await fs.promises.realpath(process.env.GEMINI_CLI_HOME || '') !== profile) throw new Error('Missing isolated Gemini profile');
  const official = await import(pathToFileURL(await findAuthModule(bundleDir)).href);
  if (typeof official.getOauthClient !== 'function' || official.AuthType?.LOGIN_WITH_GOOGLE !== 'oauth-personal') throw new Error('Unsupported official Gemini OAuth exports');
  const credentialFile = scopedPath(profile, official.Storage.getOAuthCredsPath());
  const accountFile = scopedPath(profile, official.Storage.getGoogleAccountsPath());
  // These are the Config methods used by the installed, exported initOauthClient.
  // ACP suppresses CLI consent UI; the user already explicitly started authorization in Jolu.
  const config = { getProxy: () => process.env.JOLU_AUTH_PROXY_URL || undefined, isBrowserLaunchSuppressed: () => false, getAcpMode: () => true, isInteractive: () => true };
  const client = await official.getOauthClient(official.AuthType.LOGIN_WITH_GOOGLE, config);
  const credentials = JSON.parse(credentialPayload(client.credentials));
  // The official tokens event persists asynchronously. Explicitly save the returned credentials in
  // our own profile so process exit cannot race that write. Nothing is printed to stdout/stderr.
  await fs.promises.mkdir(path.dirname(credentialFile), { recursive: true });
  await fs.promises.writeFile(credentialFile, JSON.stringify(credentials), { mode: 0o600 });
  let account = null;
  try {
    const stat = await fs.promises.stat(accountFile);
    if (stat.size <= LIMIT) {
      const active = JSON.parse(await fs.promises.readFile(accountFile, 'utf8')).active;
      if (typeof active === 'string' && active.length <= 320 && !/[\x00-\x1f\x7f]/.test(active)) account = active;
    }
  } catch { /* Unknown identity is allowed; quota verification still uses this OAuth session. */ }
  const payload = JSON.stringify({ credentials, account });
  if (Buffer.byteLength(payload, 'utf8') > LIMIT) throw new Error('OAuth credentials exceed limit');
  await fs.promises.writeFile(path.join(profile, 'jolu-session.json'), payload, { mode: 0o600 });
}

module.exports = { findAuthModule, scopedPath, credentialPayload };
if (require.main === module) {
  authorize(process.argv[2], process.argv[3]).then(() => process.exit(0), () => process.exit(1));
}
