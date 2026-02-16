#!/bin/bash
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
agent --model opus-4.6 --force "$(cat /home/hush/.openclaw/workspace/ai-rs/prompt.txt)"
