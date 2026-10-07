sed -i '' -e '5192,5202c\
    lines.push(String::new());\
    lines.push(format!("Evidence calls · {}", app.calls.len()));\
    let memories: usize = app.answer_memories.values().map(Vec::len).sum();\
    lines.push(format!("Linked memories · {memories}"));\
    lines.push(String::new());\
    lines.push("Brain context controls:".to_string());\
    lines.push(" [Strict]  Balanced  Exploratory".to_string());\
    lines.push(" Date range: All time".to_string());\
    lines.push(" Include background: No".to_string());\
    lines.push(String::new());\
    lines.push(format!("▼ Memories used ({memories})"));\
    for mem_list in app.answer_memories.values() {\
        for mem in mem_list {\
            lines.push(format!("  • {} (Admitted: strict query match)", clip_chars(&mem.text, 35)));\
        }\
    }\
    frame.render_widget(\
        Paragraph::new(lines.join("\\n"))\
            .style(theme::dim())\
            .wrap(Wrap { trim: false }),\
        info_area,\
    );\
}
