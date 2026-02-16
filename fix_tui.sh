#!/bin/bash
# 1. Update ai/src/auth/mod.rs to make each sub-provider have its own entry
sed -i 's/group: "Anthropic".into(),/group: "Anthropic (Claude)".into(),/g' ai/src/auth/mod.rs
sed -i 's/group: "Google".into(),/group: "Google (Gemini)".into(),/g' ai/src/auth/mod.rs

# 2. Add an explicit check in TUI to prompt for sub-methods if multiple exist
# Actually, the current TUI logic in handle_provider_select just picks the first method.
# We need to change how Anthropic is handled.
