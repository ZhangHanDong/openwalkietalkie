use crate::{
    testkit::{Sim, UID},
    *,
};

fn send(sim: &mut Sim, agent: &AgentId, id: &str, role: ChatRole, streaming: bool, text: &str) {
    sim.api(
        UID,
        SessionId(1),
        Request::Reply(ReplySet {
            agent: agent.clone(),
            text: text.into(),
            message: Some(MessageMeta {
                id: id.into(),
                role,
                streaming,
            }),
            speak: true,
            announce: None,
            lane: None,
        }),
    )
    .unwrap();
}

#[test]
fn streaming_upserts_history_without_speaking_or_moving_an_older_reading_window() {
    let mut sim = Sim::new();
    let agent = sim.register(UID, SessionId(1), "bot", None);
    send(
        &mut sim,
        &agent,
        "u1",
        ChatRole::User,
        false,
        &"question ".repeat(100),
    );
    assert!(!sim
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Audio(AudioCmd::Say { .. }))));
    let rev = sim.store.snapshot().agents[&agent].chat_rev;
    sim.apply(Command::Device(DeviceLine::DocLines {
        slot: 1,
        doc: DocKind::Reply,
        id: rev,
        first: 3,
    }));
    send(&mut sim, &agent, "a1", ChatRole::Assistant, true, "Hello");
    send(
        &mut sim,
        &agent,
        "a1",
        ChatRole::Assistant,
        true,
        "Hello **world**",
    );
    assert!(matches!(
        sim.line(Key::DocText(Slot(1), DocKind::Reply, rev)),
        Some(HostLine::DocText { first: 3, .. })
    ));
    assert!(!sim
        .effects
        .iter()
        .any(|e| matches!(e, Effect::Audio(AudioCmd::Say { .. }))));
    let a = &sim.store.snapshot().agents[&agent];
    assert_eq!(a.messages.len(), 2);
    assert_eq!(a.messages[1].text, "Hello **world**");
    assert!(a.reply.is_none());
    assert_eq!(a.chat_rev, rev);
    send(
        &mut sim,
        &agent,
        "a1",
        ChatRole::Assistant,
        false,
        "Hello **world**!",
    );
    let a = &sim.store.snapshot().agents[&agent];
    assert_eq!(a.messages.len(), 2);
    assert_eq!(
        a.reply.as_ref().unwrap().text.as_deref(),
        Some("Hello **world**!")
    );
    let redacted = redact(sim.store.snapshot(), &[Scope::Observe]);
    assert!(redacted.agents[&agent].messages.is_empty());
}

#[test]
fn conversation_is_bounded_and_each_message_has_its_own_markdown_document() {
    let mut sim = Sim::new();
    let agent = sim.register(UID, SessionId(1), "bot", None);
    for i in 0..66 {
        send(
            &mut sim,
            &agent,
            &format!("a{i}"),
            ChatRole::Assistant,
            false,
            "```\nunfinished fence",
        );
    }
    let a = &sim.store.snapshot().agents[&agent];
    assert_eq!(a.messages.len(), 64);
    assert_eq!(a.messages[0].message.id, "a2");
    assert!(a.chat_rev > 1);
    let Some(HostLine::DocText {
        lines,
        total,
        first,
        ..
    }) = crate::store::agent_doc_window(a, DocKind::Reply, u16::MAX, 1, None)
    else {
        panic!("window");
    };
    assert_eq!(usize::from(first) + lines.len(), usize::from(total));
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("h0") && l.contains("64")),
        "a previous open fence cannot eat a message heading"
    );
}

#[test]
fn tools_are_compact_until_tapped_and_keep_their_state_through_updates_and_resync() {
    let mut sim = Sim::new();
    let agent = sim.register(UID, SessionId(1), "bot", None);
    send(&mut sim, &agent, "t1", ChatRole::Tool, true, "**Command**\n\n```\ncargo test\n```\n\noutput");
    send(&mut sim, &agent, "t2", ChatRole::Tool, false, "**Read file**\n\nsecond tool details");
    let rev = sim.store.snapshot().agents[&agent].chat_rev;
    let window = |sim: &Sim| {
        let id = sim.store.snapshot().agents[&agent].chat_rev;
        let Some(HostLine::DocText { lines, total, .. }) = sim.line(Key::DocText(Slot(1), DocKind::Reply, id)) else { panic!("window"); };
        (total, lines)
    };
    let (total, rows) = window(&sim);
    assert_eq!(total, 4);
    assert_eq!(rows.iter().filter(|s| deck_wrap::tool_entry(s).is_some_and(|(_, open)| !open)).count(), 1);
    assert!(rows.join(" ").contains("cargo test"), "the command summary stays visible");
    assert!(!rows.join(" ").contains("output"));
    assert!(matches!(sim.line(Key::Reply(Slot(1))), Some(HostLine::Reply { text, .. }) if !text.contains("details")));
    sim.apply(Command::Device(DeviceLine::Tool { slot: 1, id: rev, index: 0 }));
    let opened = sim.store.snapshot().agents[&agent].chat_rev;
    assert_ne!(opened, rev);
    let (total, rows) = window(&sim);
    assert!(total > 4);
    assert!(rows.iter().any(|s| deck_wrap::tool_entry(s) == Some((0, true))));
    assert!(rows.join(" ").contains("cargo test"));
    // A duplicated tap cannot undo the first tap after the offsets change.
    sim.apply(Command::Device(DeviceLine::Tool { slot: 1, id: rev, index: 0 }));
    assert_eq!(sim.store.snapshot().agents[&agent].chat_rev, opened);
    send(&mut sim, &agent, "t1", ChatRole::Tool, false, "**Command · completed**\n\nall tests passed");
    assert!(window(&sim).1.join(" ").contains("all tests passed"));
    assert!(sim.store.project().iter().any(|op| matches!(op, DeviceOp::Put(_, HostLine::DocText { lines, .. }) if lines.iter().any(|s| deck_wrap::tool_entry(s) == Some((0, true))))));
    sim.apply(Command::Device(DeviceLine::Tool { slot: 1, id: opened, index: 0 }));
    assert_eq!(window(&sim).0, 4);
    send(&mut sim, &agent, "t1", ChatRole::Tool, false, &format!("**Command · completed**\n\n{}", "lots of output\n".repeat(100)));
    assert_eq!(window(&sim).0, 4, "updates never expand a collapsed tool");
    let rev = sim.store.snapshot().agents[&agent].chat_rev;
    sim.apply(Command::Device(DeviceLine::Tool { slot: 1, id: rev, index: 63 }));
    assert_eq!(sim.store.snapshot().agents[&agent].chat_rev, rev);
}

#[test]
fn returning_to_the_tail_or_reopening_a_channel_resumes_host_tail_windows() {
    let mut sim = Sim::new();
    let agent = sim.register(UID, SessionId(1), "bot", None);
    send(&mut sim, &agent, "a1", ChatRole::Assistant, true, &"line\n\n".repeat(50));
    let rev = sim.store.snapshot().agents[&agent].chat_rev;
    sim.apply(Command::Device(DeviceLine::DocLines { slot: 1, doc: DocKind::Reply, id: rev, first: 3 }));
    sim.apply(Command::Device(DeviceLine::DocLines { slot: 1, doc: DocKind::Reply, id: rev, first: u16::MAX }));
    send(&mut sim, &agent, "a1", ChatRole::Assistant, true, &"more text\n\n".repeat(60));
    let Some(HostLine::DocText { first, total, lines, .. }) = sim.line(Key::DocText(Slot(1), DocKind::Reply, rev)) else { panic!(); };
    assert_eq!(first as usize + lines.len(), total as usize);
    sim.apply(Command::Device(DeviceLine::DocLines { slot: 1, doc: DocKind::Reply, id: rev, first: 3 }));
    sim.apply(Command::Device(DeviceLine::Channel { slot: 1, seq: 2 }));
    let Some(HostLine::DocText { first, total, lines, .. }) = sim.line(Key::DocText(Slot(1), DocKind::Reply, rev)) else { panic!(); };
    assert_eq!(first as usize + lines.len(), total as usize);
}

#[test]
fn related_reads_show_an_explored_summary_and_expand_as_one_group() {
    let mut sim = Sim::new();
    let agent = sim.register(UID, SessionId(1), "bot", None);
    for (i, file) in ["voice.rs", "outbox.rs", "contract.rs", "store.rs"].into_iter().enumerate() {
        send(&mut sim, &agent, &format!("read-{i}"), ChatRole::Tool, false,
            &format!("**Read · completed**\n\n```json\n{{\"file_path\":\"/repo/src/{file}\"}}\n```\n\nprivate output {i}"));
    }
    let a = &sim.store.snapshot().agents[&agent];
    let (_, lines) = crate::store::chat_lines(a, 1, None);
    let text = lines.join(" ");
    assert!(text.contains("Explored") && text.contains("voice.rs") && text.contains("outbox.rs") && text.contains("contract.rs"));
    assert!(text.contains("1 more") && text.contains("Show details"));
    assert!(!text.contains("private output"));
    assert_eq!(lines.iter().filter(|l| deck_wrap::tool_entry(l).is_some()).count(), 1);
    let expanded = std::collections::BTreeSet::from(["read-0".to_string()]);
    let (_, lines) = crate::store::chat_lines(a, 1, Some(&expanded));
    assert!(lines.join(" ").contains("private output 3"));
}
