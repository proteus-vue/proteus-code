/**
 * The bundled instruction skill.
 *
 * A runtime skill is the idiomatic way to give the model framework-specific
 * working knowledge without inflating the deployment persona: the body is
 * loaded only when the skill is selected, and the plugin owns its lifecycle.
 *
 * @module @proteus-code/dsh-proteus-code/skill
 */

/** Structural view of `ctx.skills` (`@deepseek-ai/dsh-skill`). */
export interface SkillsSeam {
  register(skill: {
    name: string
    description: string
    whenToUse?: string
    source: string
    content: string
  }): () => void
}

/** Skill name used for both model and user discovery. */
export const PROTEUS_SKILL_NAME = 'proteus-development'

/** The instruction body, kept in sync with the Proteus framework's own docs. */
export const PROTEUS_SKILL_CONTENT = `# Developing with Proteus

Proteus is a semantic-convergence cross-platform framework: one semantic core,
any render engine. Business code targets the semantic layer only; platform
differences sink into pluggable backends.

## Core model

- **Semantic layer first.** Write standard Vue SFC and standard HTML tags.
  Never write platform DSL (no \`view\`/\`text\` mini-program tags by hand, no
  conditional compilation).
- **Everything platform-specific is a backend.** Rendering (\`ProteusRenderBackend\`),
  compilation (\`ProteusCompilerBackend\`), host containers, and native capabilities
  are all SPI seams. When something looks platform-bound, look for the seam before
  adding conditionals.
- **The IR carries the constraints.** Compiler rules are self-describing (what /
  why / when / example / verify). Prefer consulting rules over guessing.

## Working rules

1. **Check before claiming done.** Run \`proteus check\` (and the specific gate for
   the area you touched) after changes. A failing gate is a real failure.
2. **Explain, don't guess.** Use \`proteus explain <rule-id>\` for a rule's AI manual
   and \`proteus explain <file.vue>\` for the rules a file actually triggers.
3. **Respect the strict gates.** CSS, style, router, and CLI have strict modes;
   they exist to keep output backend-portable. Do not silence a gate to pass it.
4. **Layout uses fluid primitives.** Prefer \`p-fluid\` / \`p-grid\` / \`p-stack\` /
   \`p-fit\` over hand-written \`@media\` queries or hardcoded breakpoints.
5. **Capabilities over platforms.** Reach for Capability Hooks (\`useCamera\`,
   \`useLocation\`, \`usePayment\`, …) instead of calling a platform API directly.
6. **Verify with conformance, not vibes.** SPI conformance suites are the machine
   check for a backend implementation; run \`proteus conformance\` when touching one.

## Migration

When moving existing mini-program code, use \`proteus migrate mp <path>\` (idempotent,
dry-run first). Automatic tags are rewritten; callback-style APIs are marked manual
and stay your responsibility.

## Honest boundaries

Web and WeChat mini-program (Skyline) are the supported targets. The following are
planned but not yet runnable: NativeBackend (zero-native-code), host runtime and
execution carrier (AOT), full-terminal support (car/TV/watch), and arbitrary-end
access. Do not claim these work.
`

/** Register the Proteus development skill. */
export function registerProteusSkill(skills: SkillsSeam): void {
  skills.register({
    name: PROTEUS_SKILL_NAME,
    description:
      'Working knowledge for the Proteus cross-platform framework: semantic-layer discipline, strict gates, fluid layout, capability hooks, and CLI verification.',
    whenToUse:
      'Use when editing a Proteus project (a workspace with proteus.config.ts or @proteus-vue/* packages), or when the user asks about Proteus semantics, gates, or migration.',
    source: 'runtime',
    content: PROTEUS_SKILL_CONTENT,
  })
}
