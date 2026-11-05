/**
 * Shared helpers for validate-*.test.cjs (Alchemy).
 */

const { ALCHEMY_FALLBACK_WARNING } = require("./host-read-within-limit.cjs");

function cliOutput(result) {
  return `${result.stdout}${result.stderr}`;
}

function cliStderrBody(stderr) {
  return String(stderr).replace(
    new RegExp(`^${ALCHEMY_FALLBACK_WARNING.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\r?\\n`),
    "",
  );
}

module.exports = {
  ALCHEMY_FALLBACK_WARNING,
  cliOutput,
  cliStderrBody,
};
