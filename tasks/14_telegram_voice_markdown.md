# Telegram Bot Voice & Markdown Enhancements
Status: Pending

This document tracks the integration of Voice inputs/outputs and robust Markdown rendering for the native Telegram bot interface.

## 1. Dependency Updates
- [ ] Add `pulldown-cmark` (Markdown to HTML parser) to `telos_bot`
- [ ] Add `reqwest` with `multipart` feature to `telos_bot` (for API file uploads)

## 2. Configuration (`telos_core`)
- [ ] Extend `TelosConfig` with optional `openai_audio_base_url`, `openai_audio_api_key`, `tts_voice_id` to separate TTS/STT services from text generation (in case the primary text API doesn't support standard Whisper/TTS).

## 3. Markdown Formatting (`telos_bot`)
- [ ] Create an HTML renderer that converts agent-output Markdown (using `pulldown-cmark`) into Telegram's restricted HTML tags.
- [ ] Intercept image syntax `![alt](url)` and map it to Telegram's native `send_photo` method alongside the text payload.

## 4. Voice Input (Speech-to-Text)
- [ ] Handle `Voice` messages in Telegram by downloading the `.ogg` file via `bot.get_file()`.
- [ ] Send the downloaded audio binary to the OpenAI-compatible `/v1/audio/transcriptions` (STT) endpoint via `reqwest`.
- [ ] Inject the transcribed text into the `telos_daemon` run loop.
- [ ] Keep track of whether the user initiated the trace via Voice, so the system knows to reply with Voice.

## 5. Voice Output (Text-to-Speech)
- [ ] Implement TTS logic: Send the final LLM text output to the OpenAI-compatible `/v1/audio/speech` endpoint.
- [ ] Intercept `AgentFeedback::Output` and `AgentFeedback::TaskCompleted`. If the trace was initiated via voice, call the TTS endpoint.
- [ ] Send the resulting audio byte stream back to the user via `bot.send_voice()`.

## Notes / Issues
- [ ] _To be filled during implementation_
