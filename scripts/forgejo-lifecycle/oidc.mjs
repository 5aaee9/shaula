// Test-only HTTPS issuer; exercises the production daemon's JWT authentication.
import { generateKeyPairSync, sign, randomUUID, createHash } from "node:crypto";
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
  const codes = new Map();
  const encode = value => Buffer.from(JSON.stringify(value)).toString("base64url");
  const jwt = (claims, typ = "JWT") => {
    const payload = `${encode({ alg: "RS256", typ, kid: "fixture" })}.${encode(claims)}`;
    return `${payload}.${sign("RSA-SHA256", Buffer.from(payload), signing.privateKey).toString("base64url")}`;
  };
  const server = createServer({ cert: await readFile(certificate), key: await readFile(key) }, async (req, res) => {
    const requestUrl = new URL(req.url, url);
    if (requestUrl.pathname === "/authorize") {
      const p = requestUrl.searchParams;
      const redirect = p.get("redirect_uri");
      if (p.get("client_id") !== "fixture" || p.get("code_challenge_method") !== "S256"
        || !/^https:\/\/localhost:\d+\/auth\/oidc\/callback$/.test(redirect || "")) {
        res.writeHead(400); res.end(); return;
      }
      const code = randomUUID();
      codes.set(code, { nonce: p.get("nonce"), challenge: p.get("code_challenge"), redirect, at: Date.now() });
      const target = new URL(redirect);
      target.searchParams.set("code", code);
      target.searchParams.set("state", p.get("state"));
      res.writeHead(302, { location: target.toString() }); res.end(); return;
    }
    if (requestUrl.pathname === "/token" && req.method === "POST") {
      let body = "";
      for await (const part of req) { body += part; if (body.length > 8192) { res.writeHead(400); res.end(); return; } }
      const p = new URLSearchParams(body);
      const code = codes.get(p.get("code"));
      codes.delete(p.get("code"));
      if (!code || Date.now() - code.at > 60_000 || p.get("redirect_uri") !== code.redirect
        || req.headers.authorization !== `Basic ${Buffer.from("fixture:fixture-only").toString("base64")}`
        || createHash("sha256").update(p.get("code_verifier") || "").digest("base64url") !== code.challenge) {
        res.writeHead(400, { "content-type": "application/json" }); res.end('{"error":"invalid_grant"}'); return;
      }
      const now = Math.floor(Date.now() / 1000);
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ access_token: "disposable-fixture-access", token_type: "Bearer", expires_in: 3600,
        id_token: jwt({ iss: url, sub: "fixture", aud: "fixture", iat: now, exp: now + 3600, nonce: code.nonce }) }));
      return;
    }
    const body = req.url === "/jwks" ? { keys: [jwk] } : {
      issuer: url, authorization_endpoint: `${url}/authorize`, token_endpoint: `${url}/token`, jwks_uri: `${url}/jwks`,
      response_types_supported: ["code"], subject_types_supported: ["public"], id_token_signing_alg_values_supported: ["RS256"],
      token_endpoint_auth_methods_supported: ["client_secret_basic"], code_challenge_methods_supported: ["S256"],
    };
    res.writeHead(200, { "content-type": "application/json" }); res.end(JSON.stringify(body));
  });
  await new Promise(resolve => server.listen(port, "127.0.0.1", resolve));
  const now = Math.floor(Date.now() / 1000);
  const payload = `${encode({ alg: "RS256", typ: "at+jwt", kid: "fixture" })}.${encode({
    iss: url, sub: "fixture", aud: "shaula-api", client_id: "acceptance", iat: now, exp: now + 3600, jti: randomUUID(), scope: scopes,
  })}`;
  return { url, certificate, token: `${payload}.${sign("RSA-SHA256", Buffer.from(payload), signing.privateKey).toString("base64url")}`,
    close: () => new Promise(resolve => server.close(resolve)) };
}
