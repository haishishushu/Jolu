// WorkBuddy sym-v1 field envelope, verified against the installed official client.
// This helper only reads native storage; it never refreshes or saves credentials.
'use strict';
const crypto = require('node:crypto');
const fail = code => { throw new Error(code); };
const object = value => value && typeof value === 'object' && !Array.isArray(value);
function bytes(value, size) {
  if (typeof value !== 'string' || !value.length || value.length > 65536) fail('FORMAT');
  const decoded = Buffer.from(value, 'base64');
  if (decoded.toString('base64') !== value || (size !== undefined && decoded.length !== size)) fail('FORMAT');
  return decoded;
}
function decode(value) {
  if (!object(value) || value.$wbEncrypted !== 1 || Object.keys(value).sort().join() !== '$wbEncrypted,envelope') fail('UNSUPPORTED');
  const envelope = JSON.parse(bytes(value.envelope).toString('utf8'));
  if (!object(envelope) || envelope.suite !== 1) fail('UNSUPPORTED');
  if (Object.keys(envelope).sort().join() !== 'authTag,ciphertext,keyId,nonce,suite' || typeof envelope.keyId !== 'string' || !/^[a-f0-9]{16}$/.test(envelope.keyId)) fail('FORMAT');
  const nonce = bytes(envelope.nonce, 12), tag = bytes(envelope.authTag, 16), ciphertext = bytes(envelope.ciphertext);
  let key, plaintext;
  try {
    const payload = JSON.parse(process._linkedBinding('electron_browser_workbuddy_storage').loggerGet());
    if (!object(payload) || payload.version !== 1) fail('RUNTIME');
    const secret = bytes(payload.atRestSecretKey, 32), empty = secret.every(byte => byte === 0);
    secret.fill(0);
    if (empty) fail('RUNTIME');
    key = crypto.createHash('sha256').update(payload.atRestSecretKey, 'utf8').digest();
    if (crypto.createHash('sha256').update(key).digest('hex').slice(0, 16) !== envelope.keyId) fail('KEY_MISMATCH');
    const lp = text => { const data = Buffer.from(text), length = Buffer.alloc(4); length.writeUInt32BE(data.length); return Buffer.concat([length, data]); };
    const aad = Buffer.concat([Buffer.from('WB-AAD\0'), Buffer.from([1]), lp('WBEV1'), lp('sym-v1'), Buffer.from([0, 0, 0, 1]), lp(envelope.keyId), Buffer.from([2, 0, 0])]);
    const decipher = crypto.createDecipheriv('aes-256-gcm', key, nonce);
    decipher.setAAD(aad); decipher.setAuthTag(tag);
    plaintext = Buffer.concat([decipher.update(ciphertext), decipher.final()]);
    const token = plaintext.toString('utf8');
    if (token.length > 32768 || !/^[A-Za-z0-9._~+/-]+=*$/.test(token)) fail('FORMAT');
    return token;
  } finally {
    key?.fill(0);
    plaintext?.fill(0);
  }
}
let chunks = [], size = 0, replied = false;
function reply(value) {
  if (replied) return;
  replied = true;
  process.stdout.write(JSON.stringify(value));
}
process.stdin.on('data', chunk => {
  size += chunk.length;
  if (size > 65536) { chunks = []; reply({ error: 'FORMAT' }); }
  else if (!replied) chunks.push(chunk);
});
process.stdin.on('error', () => reply({ error: 'FORMAT' }));
process.stdin.on('end', () => {
  if (replied) return;
  try { reply({ token: decode(JSON.parse(Buffer.concat(chunks).toString('utf8'))) }); }
  catch (error) { reply({ error: ['FORMAT', 'UNSUPPORTED', 'RUNTIME', 'KEY_MISMATCH'].includes(error.message) ? error.message : 'DECRYPT_FAILED' }); }
  finally { chunks = []; }
});
