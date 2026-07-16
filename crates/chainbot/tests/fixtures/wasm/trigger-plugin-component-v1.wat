;; Regenerate from crates/chainbot/:
;; wasm-tools component embed wit/trigger-plugin.wit tests/fixtures/wasm/trigger-plugin-component-v1.wat -o /tmp/trigger-plugin-embedded.wasm
;; wasm-tools component new /tmp/trigger-plugin-embedded.wasm -o tests/fixtures/wasm/trigger-plugin-component-v1.wasm
(module
  (import "chainbot:trigger-plugin/trigger-host@0.1.0" "push-trigger-event"
    (func $push-trigger-event (param i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{\22event_key\22:\22component-event\22,\22occurred_at_ms\22:1710600123456,\22payload\22:{\22producer\22:\22component\22},\22checkpoint\22:\22cp-component\22}")
  (func (export "chainbot:trigger-plugin/trigger-guest@0.1.0#run-session")
    i32.const 0
    i32.const 125
    i32.const 256
    call $push-trigger-event))
