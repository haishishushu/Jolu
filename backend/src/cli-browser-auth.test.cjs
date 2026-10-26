const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

test('Gemini helper selects an exported OAuth function from the installed entry imports', async () => {
  const { findAuthModule } = require('./cli-browser-auth.cjs');
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'jolu-cli-auth-test-'));
  try {
    fs.writeFileSync(path.join(dir, 'gemini.js'), 'import "./auth-current.js";');
    fs.writeFileSync(path.join(dir, 'auth-old.js'), 'async function getOauthClient() {}\nexport { getOauthClient };');
    fs.writeFileSync(path.join(dir, 'auth-current.js'), 'async function getOauthClient() {}\nexport { getOauthClient };');
    assert.equal(await findAuthModule(dir), path.join(dir, 'auth-current.js'));
    fs.writeFileSync(path.join(dir, 'auth-current.js'), 'async function getOauthClient() {}');
    fs.unlinkSync(path.join(dir, 'auth-old.js'));
    await assert.rejects(findAuthModule(dir));
  } finally { fs.rmSync(dir, { recursive: true }); }
});

test('Gemini helper refuses credential paths outside the isolated profile', () => {
  const { scopedPath, credentialPayload } = require('./cli-browser-auth.cjs');
  const profile = path.resolve(os.tmpdir(), 'jolu-isolated-profile');
  assert.equal(scopedPath(profile, path.join(profile, '.gemini', 'oauth_creds.json')), path.join(profile, '.gemini', 'oauth_creds.json'));
  const nativeFile = path.toNamespacedPath(path.join(profile, '.gemini', 'oauth_creds.json'));
  assert.equal(scopedPath(profile, nativeFile), nativeFile);
  assert.throws(() => scopedPath(profile, path.join(profile, '..', 'real-account.json')));
  assert.throws(() => scopedPath(profile, profile));
  assert.throws(() => credentialPayload({}));
  assert.throws(() => credentialPayload({ access_token: 'x'.repeat(1024 * 1024) }));
  assert.equal(JSON.parse(credentialPayload({ access_token: 'synthetic', refresh_token: 'synthetic-refresh' })).access_token, 'synthetic');
});
