/**
 * Test stub for `@deepseek-ai/dsh-tools`.
 *
 * The real package is a runtime-only import (see src/dsh-host.d.ts), provided by
 * the DSH installation rather than by this workspace. Tests alias it here so
 * plugin registration logic can run without a harness present.
 */

/** Passthrough that returns the definition unchanged, as `defineTool` does. */
export function defineTool(definition: unknown): unknown {
  return definition
}
