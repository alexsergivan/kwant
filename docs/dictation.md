# Dictation

The message box's mic button records the person's voice and turns it into text in the text box,
where they read and edit it. Nothing reaches a bot before they press Send, and nothing is stored:
the recording stays in memory for the one call, with no attachment, file, chat message or memory.

The words come from the space's own OpenAI account, in this order (the same as hybrid recall's
embeddings, minus Ollama, which has no speech-to-text):

1. An enabled OpenAI API-key account of the space (an `api_key` profile on Codex) with a key.
2. The host's OpenAI key (Settings, else `OPENAI_API_KEY`), unless This machine is off for Codex
   in the space.

Neither is used while the OpenAI API engine is turned off. OpenAI's
`POST /v1/audio/transcriptions` gets the recording with the model `gpt-4o-mini-transcribe`, and
`whisper-1` if OpenAI says the first doesn't exist for the key. Local Whisper isn't supported yet.

## `GET /api/spaces/:id/transcribe`

Whether the mic button can work in this space. Reads only the database: no network call.

```json
{"available": true, "source": "openai", "why": null}
```

```json
{"available": false, "source": null, "why": "Add an OpenAI API key in Apps & accounts to dictate."}
```

`why` is one of:

- `Add an OpenAI API key in Apps & accounts to dictate.`
- `This machine is turned off for OpenAI in this space, so its key isn't used. Add an OpenAI API key to this space in Apps & accounts to dictate.`
- `The OpenAI API is turned off in Apps & accounts. Turn it on to dictate.`

404 `{"error": "no such space", "code": "no_space"}` for an unknown space.

## `POST /api/spaces/:id/transcribe[?lang=xx]`

The recording is the raw body. `Content-Type` is `audio/webm`, `audio/mp4`, `audio/mpeg`,
`audio/wav` or `audio/ogg` (also `audio/x-m4a`, `audio/m4a`, `audio/mp3`, `audio/x-wav`,
`audio/wave`); parameters such as `;codecs=opus` are fine. `lang` is an optional two-letter
ISO-639-1 hint (`en`, `de`). At most 25 MB.

```
POST /api/spaces/spc_default/transcribe?lang=en
Content-Type: audio/webm;codecs=opus

<recording bytes>
```

200:

```json
{"text": "Book a table for four on Friday at seven.", "source": "openai"}
```

Only the person can dictate: a bot's turn token gets 403. At most 2 recordings go to OpenAI at a
time; one more waits up to 30 s for its turn.

Every refusal is `{"error": "<a sentence for the person>", "code": "<code>"}`; 409
`no_transcriber` also has `why`. In the order they are checked:

| Status | `code` | `error` |
|---|---|---|
| 403 | none | `only the person can dictate` |
| 404 | none | `no such space` |
| 409 | none | the space is archived |
| 413 | `too_large` | `The recording is over 25 MB. Record a shorter message.` (a declared `Content-Length` over 25 MB, before any byte is read) |
| 415 | `bad_audio_type` | `Send the recording as audio/webm, audio/mp4, audio/mpeg, audio/wav or audio/ogg.` |
| 400 | `bad_lang` | `lang must be a two-letter language code such as en.` |
| 409 | `no_transcriber` | the `why` from GET, also as `why` |
| 400 | `interrupted` | `The recording was interrupted.` |
| 413 | `too_large` | as above, at the byte that passes 25 MB |
| 400 | `empty_audio` | `The recording is empty.` |
| 429 | `busy` | `Other recordings are being turned into text. Try again in a moment.` |
| 502 | `transcriber_failed` | one of the reasons below |

502 reasons (OpenAI's own error text is never passed on, since its 401 quotes part of the key):

- `The OpenAI key was refused.` (401, 403)
- `OpenAI is limiting this key, or its quota is used up. Try again in a minute.` (429)
- `The recording is too large for OpenAI.` (413)
- `OpenAI couldn't read this recording.` (400, 404, 415, 422)
- `OpenAI had a problem. Try again in a minute.` (other statuses)
- `OpenAI didn't answer within 60 seconds.`
- `OpenAI couldn't be reached.`
- `OpenAI's answer couldn't be read.`

The key never appears in an answer or a log line. The log has one line per recording
(`dictation transcribed` or `dictation failed`, with the space, size, model or reason).

## Tests

`KWNT_OPENAI_API_BASE` points dictation at another OpenAI address, only in builds with the
`test-seams` feature (as `cargo test` builds). `tests/transcribe_test.rs` uses a stub server; no
test calls the real API.
