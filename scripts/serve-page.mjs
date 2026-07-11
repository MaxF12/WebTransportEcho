import { createServer as createHttpServer } from "node:http";
import { createServer as createHttpsServer } from "node:https";
import { readFile } from "node:fs/promises";
import { X509Certificate, createHash } from "node:crypto";
import { extname, join, resolve } from "node:path";

const root = resolve(new URL("..", import.meta.url).pathname);
const publicRoot = resolve(join(root, "public"));
const indexHtml = await readFile(join(publicRoot, "index.html"), "utf8");
const cert = process.env.WT_CERT ?? join(root, "certs", "localhost.pem");
const key = process.env.WT_KEY ?? join(root, "certs", "localhost-key.pem");
const wtTargetCert = process.env.WT_TARGET_CERT ?? cert;
const port = Number(process.env.PAGE_PORT ?? 8443);
const host = process.env.PAGE_HOST ?? "localhost";
const pageTls = process.env.PAGE_TLS !== "0";
const publicOrigin = (process.env.PAGE_PUBLIC_ORIGIN ?? `https://localhost:${port}`).replace(/\/$/, "");
const wtTargetBase = process.env.WT_TARGET_BASE ?? "https://localhost:9443";
const defaultTimeoutMs = Number(process.env.WT_DEFAULT_TIMEOUT_MS ?? 5000);
const defaultBetweenCasesMs = Number(process.env.WT_BETWEEN_CASES_MS ?? 150);
const defaultMode = process.env.WT_DEFAULT_MODE === "exhaustive" ? "exhaustive" : "selected";
const defaultAutorun = process.env.WT_AUTORUN === "1";
const defaultPaths = (process.env.WT_DEFAULT_PATHS ?? "")
  .split(",")
  .map((value) => value.trim())
  .filter(Boolean);
const certPem = pageTls ? await readFile(cert, "utf8") : null;
const keyPem = pageTls ? await readFile(key) : null;
const certHashPayload = await targetCertificatePayload(wtTargetCert);
const securityHeaders = buildSecurityHeaders(indexHtml);

const mime = new Map([
  [".html", "text/html; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
]);

const requestHandler = async (req, res) => {
    const url = new URL(req.url ?? "/", `https://${req.headers.host ?? "localhost"}`);
    if (url.pathname === "/cert-sha256.json") {
      writeResponseHead(res, 200, {
        "content-type": "application/json; charset=utf-8",
        "cache-control": "no-store",
      });
      res.end(JSON.stringify(certHashPayload));
      return;
    }

    if (url.pathname === "/matrix-config.json") {
      writeResponseHead(res, 200, {
        "content-type": "application/json; charset=utf-8",
        "cache-control": "no-store",
      });
      res.end(
        JSON.stringify({
          targetBase: wtTargetBase,
          defaultTimeoutMs,
          defaultBetweenCasesMs,
          defaultPaths,
          defaultMode,
          autorun: defaultAutorun,
        }),
      );
      return;
    }

    if (url.pathname === "/healthz") {
      writeResponseHead(res, 200, {
        "content-type": "text/plain; charset=utf-8",
        "cache-control": "no-store",
      });
      res.end("ok\n");
      return;
    }

    const pathname = url.pathname === "/" ? "/index.html" : url.pathname;
    const file = resolve(join(root, "public", pathname));

    if (!file.startsWith(`${publicRoot}/`) && file !== publicRoot) {
      writeResponseHead(res, 403, { "content-type": "text/plain; charset=utf-8" });
      res.end("forbidden");
      return;
    }

    try {
      const body = await readFile(file);
      writeResponseHead(res, 200, {
        "content-type": mime.get(extname(file)) ?? "application/octet-stream",
        "cache-control": "no-store",
      });
      res.end(body);
    } catch (error) {
      writeResponseHead(res, 404, { "content-type": "text/plain; charset=utf-8" });
      res.end(`not found: ${pathname}\n`);
    }
};

const server = pageTls
  ? createHttpsServer({ cert: certPem, key: keyPem }, requestHandler)
  : createHttpServer(requestHandler);

server.listen(port, host, () => {
  console.log(`Test page: ${publicOrigin}/`);
  console.log(`Default WebTransport target: ${wtTargetBase}`);
  console.log(`WebTransport certificate SHA-256: ${certHashPayload.valueHex ?? "unavailable"}`);
  console.log(`WebTransport certificate path: ${wtTargetCert}`);
});

function writeResponseHead(res, status, headers = {}) {
  res.writeHead(status, { ...securityHeaders, ...headers });
}

async function targetCertificatePayload(path) {
  try {
    const pem = await readFile(path, "utf8");
    return {
      algorithm: "sha-256",
      valueHex: certificateSha256Hex(pem, path),
      cert: path,
      available: true,
      ...certificateMetadata(pem),
    };
  } catch (error) {
    return {
      algorithm: "sha-256",
      valueHex: null,
      cert: path,
      available: false,
      currentlyValid: false,
      isSelfSigned: false,
      publicKeyType: null,
      validityDays: null,
      hashUsable: false,
      error: `target certificate unavailable: ${error.message ?? String(error)}`,
    };
  }
}

function buildSecurityHeaders(html) {
  const scriptHashes = inlineHashes(html, "script");
  const styleHashes = inlineHashes(html, "style");
  const csp = [
    "default-src 'none'",
    "base-uri 'none'",
    "connect-src https:",
    "form-action 'none'",
    "frame-ancestors 'none'",
    "img-src 'self' data:",
    `script-src 'self' ${scriptHashes.join(" ")}`,
    `style-src 'self' ${styleHashes.join(" ")}`,
  ].join("; ");

  return {
    "content-security-policy": csp,
    "cross-origin-opener-policy": "same-origin",
    "cross-origin-resource-policy": "same-origin",
    "permissions-policy": "camera=(), microphone=(), geolocation=()",
    "referrer-policy": "no-referrer",
    "strict-transport-security": "max-age=31536000; includeSubDomains",
    "x-content-type-options": "nosniff",
    "x-frame-options": "DENY",
  };
}

function inlineHashes(html, tag) {
  const values = [];
  const expression = new RegExp(`<${tag}(?:\\s[^>]*)?>([\\s\\S]*?)<\\/${tag}>`, "gi");
  for (const match of html.matchAll(expression)) {
    const digest = createHash("sha256").update(match[1]).digest("base64");
    values.push(`'sha256-${digest}'`);
  }
  return values;
}

function certificateSha256Hex(pem, path) {
  const match = pem.match(/-----BEGIN CERTIFICATE-----([\s\S]+?)-----END CERTIFICATE-----/);
  if (!match) {
    throw new Error(`No CERTIFICATE section found in ${path}`);
  }
  const der = Buffer.from(match[1].replace(/\s+/g, ""), "base64");
  return createHash("sha256").update(der).digest("hex");
}

function certificateMetadata(pem) {
  const cert = new X509Certificate(pem);
  const validFrom = new Date(cert.validFrom);
  const validTo = new Date(cert.validTo);
  const validityDays = Math.ceil((validTo.getTime() - validFrom.getTime()) / 86_400_000);
  const publicKeyType = cert.publicKey.asymmetricKeyType;
  const isSelfSigned = cert.subject === cert.issuer && cert.verify(cert.publicKey);
  const now = Date.now();
  const currentlyValid = validFrom.getTime() <= now && now <= validTo.getTime();
  const hashUsable =
    currentlyValid && isSelfSigned && publicKeyType === "ec" && validityDays <= 14;

  return {
    subject: cert.subject,
    issuer: cert.issuer,
    validFrom: validFrom.toISOString(),
    validTo: validTo.toISOString(),
    validityDays,
    publicKeyType,
    isSelfSigned,
    currentlyValid,
    hashUsable,
  };
}
