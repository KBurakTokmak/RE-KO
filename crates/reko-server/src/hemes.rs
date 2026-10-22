//! Original1453 Valkyrie certificate menus, without later missing menu IDs.
pub(crate) fn script(file: &str) -> Option<Vec<u8>> {
    if file != "16085_Hamus.lua" {
        return None;
    }
    let mut script = String::from(
        "local NPC=16085\nif EVENT==165 then SelectMsg(UID,3,-1,4131,NPC,4286,1008,4287,1009,27,999) end\n",
    );
    for (page, certificate, first_menu, first_choice, first_confirm, first_reward, talk, missing) in [
        (
            1008,
            800180000u32,
            4288,
            1014,
            1038,
            508011441u32,
            4903,
            4921,
        ),
        (
            1009,
            800170000u32,
            4292,
            1018,
            1042,
            508013318u32,
            4907,
            4911,
        ),
    ] {
        script.push_str(&format!("if EVENT=={page} then if HowmuchItem(UID,{certificate})<1 then SelectMsg(UID,2,-1,{missing},NPC,27,999) else SelectMsg(UID,3,-1,4902,NPC"));
        for i in 0..4 {
            script.push_str(&format!(",{},{}", first_menu + i, first_choice + i));
        }
        script.push_str(",4296,165) end end\n");
        for i in 0..4 {
            script.push_str(&format!(
                "if EVENT=={} then SelectMsg(UID,2,-1,{},NPC,4297,{},4296,{page}) end\n",
                first_choice + i,
                talk + i,
                first_confirm + i
            ));
            script.push_str(&format!("if EVENT=={} then RobItem(UID,{certificate},1) GiveItem(UID,{},1) SelectMsg(UID,2,-1,4901,NPC,27,999) end\n",first_confirm+i,first_reward+i as u32));
        }
    }
    Some(script.into_bytes())
}
