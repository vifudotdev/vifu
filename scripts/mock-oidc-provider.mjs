import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { createServer } from "node:http";

const port = Number(process.env.VIFU_OIDC_TEST_PORT ?? 6800);
const baseUrl = `http://127.0.0.1:${port}`;
const issuer = `${baseUrl}/`;
const clientId = "vifu-oidc-e2e";
const clientSecret = "vifu-oidc-e2e-secret";
const keyId = "vifu-oidc-e2e-key";
const codes = new Map();
const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const publicJwk = publicKey.export({ format: "jwk" });

createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", issuer);
  if (url.pathname === "/health") return json(response, 200, { status: "ok" });
  if (url.pathname === "/.well-known/openid-configuration") {
    return json(response, 200, {
      issuer,
      authorization_endpoint: `${baseUrl}/authorize`,
      token_endpoint: `${baseUrl}/token`,
      jwks_uri: `${baseUrl}/jwks`,
      response_types_supported: ["code"],
      subject_types_supported: ["public"],
      id_token_signing_alg_values_supported: ["RS256"],
      scopes_supported: ["openid", "profile", "email"],
      token_endpoint_auth_methods_supported: ["client_secret_basic"],
      claims_supported: ["sub", "email", "email_verified", "nonce"],
    });
  }
  if (url.pathname === "/jwks") {
    return json(response, 200, { keys: [{ ...publicJwk, kid: keyId, use: "sig", alg: "RS256" }] });
  }
  if (url.pathname === "/authorize") {
    const redirectUri = url.searchParams.get("redirect_uri");
    const state = url.searchParams.get("state");
    const nonce = url.searchParams.get("nonce");
    const challenge = url.searchParams.get("code_challenge");
    if (!redirectUri || !state || !nonce || !challenge || url.searchParams.get("client_id") !== clientId) {
      return json(response, 400, { error: "invalid_request" });
    }
    const code = `code-${Date.now().toString(36)}`;
    codes.set(code, { redirectUri, nonce, challenge });
    const callback = new URL(redirectUri);
    callback.searchParams.set("code", code);
    callback.searchParams.set("state", state);
    response.statusCode = 302;
    response.setHeader("location", callback.toString());
    return response.end();
  }
  if (url.pathname === "/token" && request.method === "POST") {
    const body = new URLSearchParams(await readBody(request));
    const record = codes.get(body.get("code"));
    const authorization = request.headers.authorization ?? "";
    const expectedBasic = `Basic ${Buffer.from(`${clientId}:${clientSecret}`).toString("base64")}`;
    const verifier = body.get("code_verifier") ?? "";
    const actualChallenge = createHash("sha256").update(verifier).digest("base64url");
    if (!record || authorization !== expectedBasic || body.get("redirect_uri") !== record.redirectUri || actualChallenge !== record.challenge) {
      return json(response, 400, { error: "invalid_grant" });
    }
    codes.delete(body.get("code"));
    const now = Math.floor(Date.now() / 1000);
    const idToken = jwt({
      iss: issuer,
      aud: clientId,
      sub: "oidc-e2e-user",
      email: "oidc-admin@example.com",
      email_verified: true,
      nonce: record.nonce,
      iat: now,
      exp: now + 300,
    });
    return json(response, 200, {
      access_token: "oidc-e2e-access-token",
      token_type: "Bearer",
      expires_in: 300,
      id_token: idToken,
    });
  }
  return json(response, 404, { error: "not_found" });
}).listen(port, "127.0.0.1");

function jwt(payload) {
  const header = Buffer.from(JSON.stringify({ alg: "RS256", typ: "JWT", kid: keyId })).toString("base64url");
  const body = Buffer.from(JSON.stringify(payload)).toString("base64url");
  const input = `${header}.${body}`;
  return `${input}.${sign("RSA-SHA256", Buffer.from(input), privateKey).toString("base64url")}`;
}

async function readBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8");
}

function json(response, status, body) {
  response.statusCode = status;
  response.setHeader("content-type", "application/json");
  response.end(JSON.stringify(body));
}
