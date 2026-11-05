/**
 * ALCHEMY shared input reader for validate-findings.cjs and validate-coverage-ledger.cjs.
 * Prefer O_NOFOLLOW|O_NONBLOCK open; on hosts without those flags (typical Windows),
 * fall back to lstat + readFileSync so Full audit schema CLIs can complete.
 */

const fs = require("node:fs");
const { TextDecoder } = require("node:util");

const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

class SafeInputError extends Error {}

/** Keep in sync with cli-test-helpers.cjs strip pattern. */
const ALCHEMY_FALLBACK_WARNING =
  "WARNING: O_NOFOLLOW unavailable; Alchemy host fallback (lstat+readFileSync)";

function hasSafeInputOpen() {
  const noFollow = fs.constants.O_NOFOLLOW;
  const nonBlock = fs.constants.O_NONBLOCK;
  return (
    Number.isInteger(noFollow) &&
    noFollow !== 0 &&
    Number.isInteger(nonBlock) &&
    nonBlock !== 0
  );
}

function readFileWithinLimitFallback(file, maxBytes) {
  const stat = fs.lstatSync(file);
  if (typeof stat.isSymbolicLink === "function" && stat.isSymbolicLink()) {
    throw new SafeInputError("input must not be a symlink");
  }
  if (!stat.isFile()) {
    throw new SafeInputError("input must be a regular file");
  }
  if (stat.size > maxBytes) {
    throw new SafeInputError(`input exceeds ${maxBytes} byte limit`);
  }
  const buffer = fs.readFileSync(file);
  if (buffer.length > maxBytes) {
    throw new SafeInputError(`input exceeds ${maxBytes} byte limit`);
  }
  try {
    return UTF8_DECODER.decode(buffer);
  } catch {
    throw new SafeInputError("input is not valid UTF-8");
  }
}

function readFileWithinLimit(file, maxBytes) {
  if (!hasSafeInputOpen()) {
    console.error(ALCHEMY_FALLBACK_WARNING);
    return readFileWithinLimitFallback(file, maxBytes);
  }

  const noFollow = fs.constants.O_NOFOLLOW;
  const nonBlock = fs.constants.O_NONBLOCK;
  let descriptor;
  try {
    descriptor = fs.openSync(file, fs.constants.O_RDONLY | noFollow | nonBlock);
  } catch (error) {
    if (error && (error.code === "ELOOP" || error.code === "EMLINK")) {
      throw new SafeInputError("input must not be a symlink");
    }
    throw error;
  }
  try {
    const stat = fs.fstatSync(descriptor);
    if (!stat.isFile()) {
      throw new SafeInputError("input must be a regular file");
    }
    if (stat.size > maxBytes) {
      throw new SafeInputError(`input exceeds ${maxBytes} byte limit`);
    }

    const chunks = [];
    const buffer = Buffer.allocUnsafe(64 * 1024);
    let bytesRead = 0;
    while (true) {
      const count = fs.readSync(descriptor, buffer, 0, buffer.length, null);
      if (count === 0) break;
      bytesRead += count;
      if (bytesRead > maxBytes) {
        throw new SafeInputError(`input exceeds ${maxBytes} byte limit`);
      }
      chunks.push(Buffer.from(buffer.subarray(0, count)));
    }
    try {
      return UTF8_DECODER.decode(Buffer.concat(chunks, bytesRead));
    } catch {
      throw new SafeInputError("input is not valid UTF-8");
    }
  } finally {
    fs.closeSync(descriptor);
  }
}

module.exports = {
  ALCHEMY_FALLBACK_WARNING,
  SafeInputError,
  hasSafeInputOpen,
  readFileWithinLimit,
  readFileWithinLimitFallback,
};
