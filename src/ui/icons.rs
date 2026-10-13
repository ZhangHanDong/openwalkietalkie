//! Lucide glyph IDs are the stable OWT1 protocol catalogue. Do not reorder.
//! Source, codepoints and licenses: assets/lucide/.
pub const SIZE: usize = 40;
pub const GLYPHS: [&[u8; SIZE * SIZE]; 34] = [
    include_bytes!("../../assets/lucide/blank.alpha"), // 0
    include_bytes!("../../assets/lucide/zap.alpha"),   // 1
    include_bytes!("../../assets/lucide/circle-check.alpha"), // 2
    include_bytes!("../../assets/lucide/circle-x.alpha"), // 3
    include_bytes!("../../assets/lucide/split.alpha"), // 4
    include_bytes!("../../assets/lucide/mic.alpha"),   // 5
    include_bytes!("../../assets/lucide/brush-cleaning.alpha"), // 6
    include_bytes!("../../assets/lucide/bot.alpha"),   // 7
    include_bytes!("../../assets/lucide/chevron-up.alpha"), // 8
    include_bytes!("../../assets/lucide/bug.alpha"),   // 9
    include_bytes!("../../assets/lucide/terminal.alpha"), // 10
    include_bytes!("../../assets/lucide/download.alpha"), // 11
    include_bytes!("../../assets/lucide/trash-2.alpha"), // 12
    include_bytes!("../../assets/lucide/square-pen.alpha"), // 13
    include_bytes!("../../assets/lucide/mouse-pointer-2.alpha"), // 14
    include_bytes!("../../assets/lucide/star.alpha"),  // 15
    include_bytes!("../../assets/lucide/file-diff.alpha"), // 16
    include_bytes!("../../assets/lucide/play.alpha"),  // 17
    include_bytes!("../../assets/lucide/git-branch.alpha"), // 18
    include_bytes!("../../assets/lucide/git-merge.alpha"), // 19
    include_bytes!("../../assets/lucide/git-pull-request.alpha"), // 20
    include_bytes!("../../assets/lucide/git-pull-request-draft.alpha"), // 21
    include_bytes!("../../assets/lucide/flask-conical.alpha"), // 22
    include_bytes!("../../assets/lucide/party-popper.alpha"), // 23
    include_bytes!("../../assets/lucide/clock.alpha"), // 24
    include_bytes!("../../assets/lucide/brain.alpha"), // 25
    include_bytes!("../../assets/lucide/settings.alpha"), // 26
    include_bytes!("../../assets/lucide/folder-plus.alpha"), // 27
    include_bytes!("../../assets/lucide/cloud-upload.alpha"), // 28
    include_bytes!("../../assets/lucide/layout-grid.alpha"), // 29
    include_bytes!("../../assets/lucide/sparkles.alpha"), // 30
    include_bytes!("../../assets/lucide/circle-help.alpha"), // 31
    include_bytes!("../../assets/lucide/rocket.alpha"), // 32
    include_bytes!("../../assets/lucide/send.alpha"),  // 33
];
pub const CHEVRON: &[u8; SIZE * SIZE] = GLYPHS[8];
