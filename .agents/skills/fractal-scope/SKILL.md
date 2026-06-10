version: 1

l3_file_header:
  enabled: true
  include:
    - "crates/**/*.rs"
    - "official-plugins/**/src/*.rs"
    - "official-plugins/**/src/**/*.rs"
    - "interface/**/src/*.ts"
    - "interface/**/src/**/*.ts"
    - "interface/**/src/*.tsx"
    - "interface/**/src/**/*.tsx"
    - "interface/**/src/*.astro"
    - "interface/**/src/**/*.astro"
    - "interface/**/src/*.css"
    - "interface/**/src/**/*.css"
  exclude:
    - "crates/**/tests/fixtures/**"
    - "docs/**"
    - "examples/**"
    - ".agents/**"
    - "**/target/**"
    - "**/node_modules/**"
    - "**/dist/**"
    - "**/.astro/**"

l2_folder_manifest:
  enabled: true
  include:
    - "crates/AGENTS.md"
    - "crates/**/AGENTS.md"
    - "docs/AGENTS.md"
    - "docs/**/AGENTS.md"
    - "examples/AGENTS.md"
    - "examples/**/AGENTS.md"
    - "interface/AGENTS.md"
    - "interface/**/AGENTS.md"
    - "official-plugins/AGENTS.md"
    - "official-plugins/**/AGENTS.md"
  exclude:
    - ".agents/**"
    - "**/target/**"
    - "**/node_modules/**"
    - "**/dist/**"
    - "**/.astro/**"

spec_output:
  mode: ask  # ask | always_file | always_inline
