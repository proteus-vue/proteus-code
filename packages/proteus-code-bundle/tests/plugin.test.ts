import { describe, expect, it, vi } from 'vitest'
import { ALLOWED_SUBCOMMANDS } from '../src/tools.ts'
import { apply, Config } from '../src/index.ts'
import type { Config as ProteusConfig } from '../src/index.ts'

/** Minimal Cordis-shaped context that records every registration. */
function makeCtx() {
  const tools: { name: string; description: string; parameters: Record<string, unknown> }[] = []
  const commands: { name: string; description: string }[] = []
  const skills: { name: string; source: string; content: string }[] = []
  const injected: string[] = []
  const listeners: { event: string }[] = []
  const indexTaps: ((html: string) => string)[] = []

  const ctx = {
    tools: {
      register: (definition: unknown) => {
        const d = definition as { name: string; description: string; parameters: Record<string, unknown> }
        tools.push({ name: d.name, description: d.description, parameters: d.parameters })
      },
    },
    get: () => undefined,
    on: (event: string) => {
      listeners.push({ event })
      return () => {}
    },
    inject: (deps: string[], callback: (scope: unknown) => void) => {
      injected.push(...deps)
      const scope: Record<string, unknown> = {}
      if (deps.includes('commands')) {
        scope.commands = {
          register: (definition: unknown) => {
            const d = definition as { name: string; description: string }
            commands.push({ name: d.name, description: d.description })
          },
        }
      }
      if (deps.includes('skills')) {
        scope.skills = {
          register: (skill: unknown) => {
            const s = skill as { name: string; source: string; content: string }
            skills.push({ name: s.name, source: s.source, content: s.content })
          },
        }
      }
      if (deps.includes('webServer')) {
        scope.webServer = {
          tapIndex: (transform: (html: string) => string) => {
            indexTaps.push(transform)
            return () => {}
          },
        }
      }
      callback(scope)
    },
  }

  return { ctx, tools, commands, skills, injected, listeners, indexTaps }
}

/** Apply the plugin with defaults filled in the way Schemastery would. */
function applyWith(overrides: Partial<ProteusConfig> = {}) {
  const harness = makeCtx()
  const config = {
    timeoutMs: 120_000,
    enableCommands: true,
    enableSkill: true,
    ...overrides,
  } as ProteusConfig
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  apply(harness.ctx as any, config)
  return harness
}

describe('plugin registration', () => {
  it('registers the full Proteus tool surface with descriptions', () => {
    const { tools } = applyWith()
    expect(tools.map((t) => t.name).sort()).toEqual([
      'proteus_audit',
      'proteus_build',
      'proteus_check',
      'proteus_cli',
      'proteus_conformance',
      'proteus_explain',
      'proteus_health',
      'proteus_migrate_mp',
      'proteus_policy',
      'proteus_rules',
    ])
    for (const tool of tools) {
      expect(tool.description.length).toBeGreaterThan(20)
      expect(tool.parameters).toBeTypeOf('object')
    }
  })

  it('registers the slash commands through the injected commands service', () => {
    const { commands, injected } = applyWith()
    expect(injected).toContain('commands')
    expect(commands.map((c) => c.name)).toEqual([
      'proteus-health',
      'proteus-check',
      'proteus-build',
      'proteus-rules',
      'proteus-explain',
    ])
  })

  it('registers the Proteus development skill as a runtime contribution', () => {
    const { skills, injected } = applyWith()
    expect(injected).toContain('skills')
    expect(skills).toHaveLength(1)
    expect(skills[0]?.name).toBe('proteus-development')
    expect(skills[0]?.source).toBe('runtime')
    expect(skills[0]?.content).toContain('semantic')
  })

  it('omits optional surfaces when disabled', () => {
    const { commands, skills, tools } = applyWith({ enableCommands: false, enableSkill: false })
    expect(commands).toHaveLength(0)
    expect(skills).toHaveLength(0)
    expect(tools).toHaveLength(10)
  })

  it('installs the execution-policy guard on the pre-execute waterfall', () => {
    const { listeners } = applyWith()
    expect(listeners.map((l) => l.event)).toContain('tools/pre-execute')
  })

  it('contributes the brand index transform through the web server', () => {
    const { indexTaps } = applyWith()
    expect(indexTaps).toHaveLength(1)
    const html = indexTaps[0]!('<html><head><title>DeepSeek Harness</title></head><body></body></html>')
    expect(html).toContain('<title>proteus code</title>')
    expect(html).toContain('data-proteus-code="brand"')
  })

  it('skips brand assets when disabled', () => {
    const { indexTaps } = applyWith({ enableBrandAssets: false })
    expect(indexTaps).toHaveLength(0)
  })

  it('exposes a schema for plugin configuration', () => {
    expect(Config).toBeDefined()
    expect(typeof Config).toBe('object')
  })
})

describe('escape hatch policy', () => {
  it('allows only known Proteus subcommands', () => {
    expect(ALLOWED_SUBCOMMANDS.has('health')).toBe(true)
    expect(ALLOWED_SUBCOMMANDS.has('css:check')).toBe(true)
    expect(ALLOWED_SUBCOMMANDS.has('rm -rf /')).toBe(false)
    expect(ALLOWED_SUBCOMMANDS.has('install')).toBe(false)
  })
})
