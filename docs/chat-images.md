# Images in chat

Claude and Codex image attachments and public tool image results appear in the
Mac app and the device's conversation view. Codex image viewing and image
generation items are included. An image-only message is retained even when it
has no accompanying text. Thinking/reasoning content is not inspected.

In the app, the four latest images load automatically. Click a preview to open
it in the detail pane; **Back to chat** returns to the conversation. Older
images have a **Load image** button. The image caption identifies its sender,
filename when available, and original dimensions.

On the device, tap a blue image row, including those below collapsed tool
groups. Loading progress is shown while the host transfers the preview.
**CANCEL** or the on-screen **Close** returns to chat. Push-to-talk pauses new
image chunk requests. A disconnected host, expired preview, or failed transfer
shows **Preview unavailable** with a retry hint. App and device firmware both
need the image-support build.

## Supported content and limits

- PNG, JPEG, WebP, and the first frame of GIF images, supplied as base64,
  image data URLs, or an explicit absolute local image attachment path.
- Up to four images per message. No automatic remote URL downloads, Markdown
  image-link resolution, SVG rendering, or image uploads from the device.
- Source files up to 8 MiB and 8192 pixels per dimension; decoder allocations
  are capped at 64 MiB. JPEG orientation is respected.
- App previews fit within 1024 × 1024; device previews fit within 128 × 128,
  with a 256-color palette. The original aspect ratio is preserved.
- Previews are kept in a process-local cache limited to 64 entries and 8 MiB.
  Original image bytes are discarded after conversion. Older previews can
  expire, and restarting the daemon clears the cache; retained provider history
  repopulates it when parsed. Claude's transcript reader has a bounded 12 MiB
  tail, so older attachments outside that tail are not recovered.
- The app retains at most eight previews and releases their decoded GPUI
  assets when evicting them, changing sessions, or closing the window.
- The device holds one 16.5 KiB preview buffer. It does not allocate a full
  480 × 480 image framebuffer.

## Host API

`MessageMeta.images` contains only `{id, label, width, height}` references.
Image data never enters the snapshot or event ring. References are removed
from events and snapshots when chat content is redacted.

`image.get {agent, image, offset?}` requires `observe:content`. The image must
belong to a currently retained message of the named agent. It returns
`{mime:"image/png", offset, total, data}`, with base64 data for at most 48 KiB
of PNG bytes. Start at offset zero and advance by the decoded byte count until
`total` is reached. Invalid offsets, unknown images, and expired previews fail
without reading an arbitrary path. Chunking keeps responses below the existing
256 KiB frame limit used by older clients.

## Device wire

The device requests `{"m":"image","agent":1,"r":1,"i":0,"o":0}` on
channel 5. `r` is the conversation revision, `i` the flattened image index,
and `o` the byte offset. The host checks the slot, revision, and image index.

The response has the same fields plus `n` (total bytes), `w`, `h`, and base64
`data`. Each chunk carries at most 384 decoded bytes. The payload is 512 bytes
of little-endian RGB565 palette entries followed by `w*h` palette indices.
Zero dimensions and total indicate an unavailable preview. No image bytes are
sent without a request, so older devices do not receive unknown image packets.

The device accepts only matching, contiguous chunks with consistent dimensions
and total size. It requests the next chunk after accepting the previous one,
retries a request after six seconds, and stops after three attempts. Closing
the viewer or changing the conversation invalidates pending chunks.

Deterministic previews: `tools/preview-all.py --board hw-a channel-images
image-preview image-loading image-error`, and app `--shots` scenes `images`
and `image-expanded` (synthetic data, no service connection).
