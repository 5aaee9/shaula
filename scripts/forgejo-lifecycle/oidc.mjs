// Test-only HTTPS issuer; exercises the production daemon's JWT authentication.
import { generateKeyPairSync, sign, randomUUID } from "node:crypto";
import { createServer } from "node:https";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { command, freePort } from "./support.mjs";

export const scopes = "fleet.read fleet.write fleet.retire template.read template.publish auth.read auth.write auth.retire logs.read";
export async function issuer(directory) {
  const certificate = join(directory, "oidc.pem");
  const key = join(directory, "oidc.key");
  await command("openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", certificate,
    "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=DNS:localhost", "-addext", "basicConstraints=critical,CA:FALSE"]);
  const signing = generateKeyPairSync("rsa", { modulusLength: 2048 });
  const jwk = { ...signing.publicKey.export({ format: "jwk" }), kid: "fixture", use: "sig", alg: "RS256" };
  const port = await freePort();
  const url = `https://localhost:${port}`;
  const server = createServer({ cert: await readFile(certificate), key: await readFile(key) }, (req, res) => {
    const body = req.url === "/jwks" ? { keys: [jwk] } : {
      issuer: url, authorization_endpoint: `${url}/authorize`, token_endpoint: `${url}/token`, jwks_uri: `${url}/jwks`,
      response_types_supported: ["code"], subject_types_supported: ["public"], id_token_signing_alg_values_supported: ["RS256"],
      token_endpoint_auth_methods_supported: ["client_secret_basic"], code_challenge_methods_supported: ["S256"],
    };
    res.writeHead(200, { "content-type": "application/json" }); res.end(JSON.stringify(body));
  });
  await new Promise(resolve => server.listen(port, "127.0.0.1", resolve));
  const now = Math.floor(Date.now() / 1000);
  const encode = value => Buffer.from(JSON.stringify(value)).toString("base64url");
  const payload = `${encode({ alg: "RS256", typ: "at+jwt", kid: "fixture" })}.${encode({
    iss: url, sub: "fixture", aud: "shaula-api", client_id: "acceptance", iat: now, exp: now + 3600, jti: randomUUID(), scope: scopes,
  })}`;
  return { url, certificate, token: `${payload}.${sign("RSA-SHA256", Buffer.from(payload), signing.privateKey).toString("base64url")}`,
    close: () => new Promise(resolve => server.close(resolve)) };
}
