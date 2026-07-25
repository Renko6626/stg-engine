//! viewer —— WebSocket 实时查看器（spec 2026-07-23）：线格式 v1 编码 + 输入映射（刀 1）
//! 与 serve 服务/内嵌 canvas 页（本刀，刀 2）。断层线以上；数据只经 stg-core 既有读出口，
//! core 零改动——新依赖 `tungstenite` 只进本 crate（`cargo tree -p stg-core` 防火墙断言）。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use stg_core::World;
use stg_core::input::InputFrame;

pub(crate) const WIRE_VERSION: u8 = 1;

/// 扫存活位字逐 index 回调（A9 批量消费姿势；尾位超 cap 部分核侧保证为零）。
fn for_each_alive(words: &[u64], mut f: impl FnMut(usize)) {
    for (wi, &w) in words.iter().enumerate() {
        let mut bits = w;
        while bits != 0 {
            f(wi * 64 + bits.trailing_zeros() as usize);
            bits &= bits - 1;
        }
    }
}

fn alive_count(words: &[u64]) -> u32 {
    words.iter().map(|w| w.count_ones()).sum()
}

/// 通道 A/B 快照 → 线格式 v1（spec §2.3：小端、顺序拼接、无填充）。
///
/// 玩家段的生死/无敌两字段实名为 `PlayerState::life_state`/`invuln`（brief 草稿写
/// `life`/`invuln`，`life` 按实况对齐为 `life_state`，`invuln` 本就同名——机械对齐，不算
/// 偏离；`life_state` 语义见 `stg_core::player` 的 `LIFE_*` 常量）。
pub(crate) fn encode_frame(w: &World) -> Vec<u8> {
    let v = w.view();
    let mut out = Vec::with_capacity(16 * 1024);
    out.push(WIRE_VERSION);
    out.extend_from_slice(&w.frame().to_le_bytes());

    // 玩家段（v1 恒 1 条：玩家 0）
    out.push(1u8);
    let p = &v.players()[0];
    out.extend_from_slice(&p.x.raw().to_le_bytes());
    out.extend_from_slice(&p.y.raw().to_le_bytes());
    out.push(p.life_state);
    out.extend_from_slice(&p.invuln.to_le_bytes());

    // boss 公告板两槽
    out.push(2u8);
    for slot in v.boss_ui() {
        out.push(slot.active);
        out.extend_from_slice(&slot.hp_ratio.raw().to_le_bytes());
        out.extend_from_slice(&slot.spell_id.to_le_bytes());
        out.extend_from_slice(&slot.timer_frames.to_le_bytes());
    }

    // 弹池（x,y,sprite = 10 B/条）
    let b = v.bullets();
    out.extend_from_slice(&(alive_count(b.alive_words()) as u16).to_le_bytes());
    for_each_alive(b.alive_words(), |i| {
        out.extend_from_slice(&b.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&b.y()[i].raw().to_le_bytes());
        out.extend_from_slice(&b.sprite()[i].to_le_bytes());
    });

    // 自机弹（同 10 B）
    let s = v.shots();
    out.extend_from_slice(&(alive_count(s.alive_words()) as u16).to_le_bytes());
    for_each_alive(s.alive_words(), |i| {
        out.extend_from_slice(&s.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&s.y()[i].raw().to_le_bytes());
        out.extend_from_slice(&s.sprite()[i].to_le_bytes());
    });

    // 敌（x,y,sprite,hp_pct = 11 B）
    let e = v.enemies();
    out.extend_from_slice(&(alive_count(e.alive_words()) as u16).to_le_bytes());
    for_each_alive(e.alive_words(), |i| {
        out.extend_from_slice(&e.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&e.y()[i].raw().to_le_bytes());
        out.extend_from_slice(&e.sprite()[i].to_le_bytes());
        let pct = (e.hp()[i].max(0) as i64 * 255) / (e.hp_max()[i].max(1) as i64);
        out.push(pct.min(255) as u8);
    });

    // 道具（x,y,item_type = 9 B）
    let it = v.items();
    out.extend_from_slice(&(alive_count(it.alive_words()) as u16).to_le_bytes());
    for_each_alive(it.alive_words(), |i| {
        out.extend_from_slice(&it.x()[i].raw().to_le_bytes());
        out.extend_from_slice(&it.y()[i].raw().to_le_bytes());
        out.push(it.item_type()[i]);
    });

    // 通道 B（id,seq,args[6] = 28 B）
    let reqs = w.take_requests();
    out.extend_from_slice(&(reqs.len() as u16).to_le_bytes());
    for r in reqs {
        out.extend_from_slice(&r.id.to_le_bytes());
        out.extend_from_slice(&r.seq.to_le_bytes());
        for a in r.args {
            out.extend_from_slice(&a.to_le_bytes());
        }
    }
    out
}

/// 浏览器 u32 掩码 → 玩家 0 InputFrame（位布局即 `BTN_*`，spec §2.4；
/// BOMB 的 Edge 语义由引擎 `decode_input` 自理，这里只送电平）。
pub(crate) fn mask_to_input(frame: u32, mask: u32) -> InputFrame {
    let mut input = InputFrame::empty(frame);
    input.actions[0].buttons = mask;
    input
}

const INDEX_HTML: &str = include_str!("../viewer/index.html");

/// `dump --out FILE [--frames 900] [--seed 1]`——离线录一局(脚本自动打:射击 + 每秒左右
/// 横移,golden 场景 2 同款),每帧写 `u32 len(LE) + 线格式 v1 帧`。回放页/调试工具消费
/// (follow-ups F3 预告的 dump 形态)。
pub(crate) fn cmd_dump(rest: &[String]) -> ExitCode {
    use stg_core::input::{BTN_LEFT, BTN_RIGHT, BTN_SHOT};
    let mut out_path: Option<String> = None;
    let mut frames: u32 = 900;
    let mut seed: u64 = 1;
    let mut i = 0;
    while i < rest.len() {
        match (rest[i].as_str(), rest.get(i + 1)) {
            ("--out", Some(v)) => {
                out_path = Some(v.clone());
                i += 2;
            }
            ("--frames", Some(v)) => {
                frames = v.parse().expect("--frames 要 u32");
                i += 2;
            }
            ("--seed", Some(v)) => {
                seed = v.parse().expect("--seed 要 u64");
                i += 2;
            }
            (a, _) => {
                eprintln!("dump: 未知参数 {a}（支持 --out/--frames/--seed）");
                return ExitCode::from(2);
            }
        }
    }
    let Some(path) = out_path else {
        eprintln!("dump: 缺 --out FILE");
        return ExitCode::from(2);
    };
    let (mut w, image, _boss) = crate::build_rainbow_world(seed);
    let mut buf = Vec::new();
    for f in 0..frames {
        let mask = BTN_SHOT
            | if (f / 60) % 2 == 0 {
                BTN_LEFT
            } else {
                BTN_RIGHT
            };
        let input = mask_to_input(f, mask);
        stg_core::step(&mut w, &stg_core::tables::TABLES_V0, &image, &input);
        let fb = encode_frame(&w);
        buf.extend_from_slice(&(fb.len() as u32).to_le_bytes());
        buf.extend_from_slice(&fb);
    }
    match std::fs::write(&path, &buf) {
        Ok(()) => {
            eprintln!("dump: {frames} 帧 → {path}（{} B）", buf.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("dump: 写 {path} 失败：{e}");
            ExitCode::from(1)
        }
    }
}

/// `serve [--port 8611] [--seed 1]`——单端口：HTTP GET 回内嵌页，WS 升级进 60Hz 游戏循环。
/// 单客户端串行伺候；断开/刷新 = 下一局新 World（天然 restart）。
pub(crate) fn cmd_serve(rest: &[String]) -> ExitCode {
    let mut port: u16 = 8611;
    let mut seed: u64 = 1;
    let mut i = 0;
    while i < rest.len() {
        match (rest[i].as_str(), rest.get(i + 1)) {
            ("--port", Some(v)) => {
                port = v.parse().expect("--port 要 u16");
                i += 2;
            }
            ("--seed", Some(v)) => {
                seed = v.parse().expect("--seed 要 u64");
                i += 2;
            }
            (a, _) => {
                eprintln!("serve: 未知参数 {a}（支持 --port/--seed）");
                return ExitCode::from(2);
            }
        }
    }
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("serve: 绑定 127.0.0.1:{port} 失败：{e}");
            return ExitCode::from(1);
        }
    };
    eprintln!(
        "viewer 就绪：http://localhost:{port}   （远程盒子上用 `ssh -L {port}:localhost:{port} <box>` 转发）"
    );
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                if let Err(e) = handle_conn(s, seed) {
                    eprintln!("serve: 连接结束（{e}），等待下一个……");
                }
            }
            Err(e) => eprintln!("serve: accept 失败：{e}"),
        }
    }
    ExitCode::SUCCESS
}

/// peek 至请求头读齐（`\r\n\r\n`）或 1KB/超时（~250ms）为止再判——分包到达的握手不误判
/// 成 HTTP（不消费字节；tungstenite 随后自读完整握手）。
fn is_ws_upgrade(stream: &TcpStream) -> bool {
    let old = stream.read_timeout().ok().flatten();
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut buf = [0u8; 1024];
    let mut seen = 0usize;
    loop {
        if let Ok(n) = stream.peek(&mut buf) {
            seen = n;
            let head = &buf[..n];
            if n >= 1024 || head.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = stream.set_read_timeout(old);
    String::from_utf8_lossy(&buf[..seen])
        .to_ascii_lowercase()
        .contains("upgrade: websocket")
}

fn handle_conn(mut stream: TcpStream, seed: u64) -> Result<(), Box<dyn std::error::Error>> {
    if !is_ws_upgrade(&stream) {
        // 读走请求（尽力而为）再回页，部分浏览器不读完请求就写会 RST
        let mut sink = [0u8; 2048];
        let _ = stream.read(&mut sink);
        let body = INDEX_HTML.as_bytes();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;
        return Ok(());
    }
    let mut ws = tungstenite::accept(stream)?;
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_millis(1)))?;

    let (mut w, image, _boss) = crate::build_rainbow_world(seed);
    let mut mask = 0u32;
    let tick = Duration::from_nanos(16_666_667); // 60 Hz（I6 固定步；节拍器住表现侧）
    let mut next = Instant::now(); // 首步即刻，此后每步 += tick（起步无双拍空隙）
    loop {
        // 排空待读消息，取最新掩码
        loop {
            match ws.read() {
                Ok(tungstenite::Message::Binary(b)) if b.len() == 4 => {
                    mask = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                }
                Ok(tungstenite::Message::Close(_)) => return Ok(()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        // 连补上限 3 步防死亡螺旋；每步都推流（浏览器 rAF 自会合帧）
        let mut stepped = 0;
        loop {
            let input = mask_to_input(w.frame(), mask);
            stg_core::step(&mut w, &stg_core::tables::TABLES_V0, &image, &input);
            // 限（Minor，M3 前不修）：ws.send 无写超时——单客户端本地查看器工具，对端
            // 卡死会阻塞本线程；当前应对是杀进程重启，M3 泛化成多客户端/联机时需重议。
            ws.send(tungstenite::Message::Binary(encode_frame(&w).into()))?;
            stepped += 1;
            next += tick;
            if next > Instant::now() || stepped >= 3 {
                break;
            }
        }
        if stepped >= 3 {
            next = Instant::now() + tick; // 落后过多：重锚，弃补
        }
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stg_core::input::{BTN_SHOT, BTN_SLOW, InputFrame};
    use stg_core::step;
    use stg_core::tables::TABLES_V0;

    fn read_u8(b: &[u8], o: &mut usize) -> u8 {
        let v = b[*o];
        *o += 1;
        v
    }
    fn read_u16(b: &[u8], o: &mut usize) -> u16 {
        let v = u16::from_le_bytes([b[*o], b[*o + 1]]);
        *o += 2;
        v
    }
    fn read_u32(b: &[u8], o: &mut usize) -> u32 {
        let v = u32::from_le_bytes(b[*o..*o + 4].try_into().unwrap());
        *o += 4;
        v
    }
    fn read_i32(b: &[u8], o: &mut usize) -> i32 {
        let v = i32::from_le_bytes(b[*o..*o + 4].try_into().unwrap());
        *o += 4;
        v
    }

    /// 开局静态帧逐字段手工解码（判别：错位/漏段/字节序错任一即红）。
    ///
    /// boss 段两槽在开局天然全零（frame 0 无 boss 公告板写入）——若逐字段断言仍去比对
    /// 「实况值」，`encode_frame` 内 `spell_id`/`timer_frames` 的写序被交换也会全绿
    /// （零 == 零，判别式盲区）。这里编码**前**给两槽灌互异非零值，断言比对这些字面量，
    /// 才真正判别 boss 段内部字段顺序/宽度/字节序。
    #[test]
    fn encode_frame_layout_v1_static_open() {
        let (mut w, _image, _boss) = crate::build_rainbow_world(42);
        w.body.boss_set(
            0,
            stg_core::boss::BossUiSlot {
                active: 1,
                hp_ratio: stg_core::math::Fx::from_raw(49_152), // 0.75
                spell_id: 7,
                timer_frames: 900,
                ..Default::default()
            },
        );
        w.body.boss_set(
            1,
            stg_core::boss::BossUiSlot {
                active: 2,
                hp_ratio: stg_core::math::Fx::from_raw(12_345),
                spell_id: 42,
                timer_frames: 1234,
                ..Default::default()
            },
        );
        let buf = encode_frame(&w);
        let mut o = 0;
        assert_eq!(read_u8(&buf, &mut o), WIRE_VERSION);
        assert_eq!(read_u32(&buf, &mut o), 0, "未 step，frame=0");
        assert_eq!(read_u8(&buf, &mut o), 1, "player_count");
        assert_eq!(read_i32(&buf, &mut o), w.view().players()[0].x.raw());
        assert_eq!(read_i32(&buf, &mut o), w.view().players()[0].y.raw());
        let _life = read_u8(&buf, &mut o);
        let _invuln = read_u16(&buf, &mut o);
        assert_eq!(read_u8(&buf, &mut o), 2, "boss_slots");
        let expect: [(u8, i32, u16, u16); 2] = [(1, 49_152, 7, 900), (2, 12_345, 42, 1234)];
        for (slot, ui) in expect.iter().enumerate() {
            assert_eq!(read_u8(&buf, &mut o), ui.0, "boss_ui[{slot}].active");
            assert_eq!(read_i32(&buf, &mut o), ui.1, "boss_ui[{slot}].hp_ratio");
            assert_eq!(read_u16(&buf, &mut o), ui.2, "boss_ui[{slot}].spell_id");
            assert_eq!(read_u16(&buf, &mut o), ui.3, "boss_ui[{slot}].timer_frames");
        }
        assert_eq!(read_u16(&buf, &mut o), 0, "开局零弹");
        assert_eq!(read_u16(&buf, &mut o), 0, "零自机弹");
        assert_eq!(read_u16(&buf, &mut o), 1, "唯 boss 一敌");
        let ex = read_i32(&buf, &mut o);
        let ey = read_i32(&buf, &mut o);
        let boss_i = w.view().enemies().iter_alive().next().unwrap();
        assert_eq!(ex, w.view().enemies().x()[boss_i].raw());
        assert_eq!(ey, w.view().enemies().y()[boss_i].raw());
        let _sprite = read_u16(&buf, &mut o);
        assert_eq!(read_u8(&buf, &mut o), 255, "hp==hp_max → 255");
        assert_eq!(read_u16(&buf, &mut o), 0, "零道具");
        assert_eq!(read_u16(&buf, &mut o), 0, "零请求");
        assert_eq!(o, buf.len(), "无多余尾字节");
    }

    /// 跑起来后：总长 = 头 + Σ(计数×记录宽) 精确对账（判别：任何记录宽错即红）。
    #[test]
    fn encode_frame_width_accounting_after_steps() {
        let (mut w, image, _boss) = crate::build_rainbow_world(42);
        for f in 0..240u32 {
            step(&mut w, &TABLES_V0, &image, &InputFrame::empty(f));
        }
        let v = w.view();
        let (nb, ns, ne, ni) = (
            v.bullets().iter_alive().count(),
            v.shots().iter_alive().count(),
            v.enemies().iter_alive().count(),
            v.items().iter_alive().count(),
        );
        let nr = w.take_requests().len();
        assert!(nb > 0, "240 帧风铃卡应已起弹（场景前提）");
        let buf = encode_frame(&w);
        let expect = (1 + 4)
            + (1 + 11)
            + (1 + 18)
            + (2 + nb * 10)
            + (2 + ns * 10)
            + (2 + ne * 11)
            + (2 + ni * 9)
            + (2 + nr * 28);
        assert_eq!(buf.len(), expect);
    }

    #[test]
    fn mask_to_input_places_buttons_on_player0() {
        let input = mask_to_input(7, BTN_SHOT | BTN_SLOW);
        assert_eq!(input.actions[0].buttons, BTN_SHOT | BTN_SLOW);
        assert_eq!(input.actions[1].buttons, 0, "玩家 1 不受掩码影响");
    }

    #[test]
    fn is_ws_upgrade_discriminates_http_vs_ws() {
        // 用本机回环真连一把：起监听线程，分别发 HTTP GET 与含 Upgrade 头的请求
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (s1, _) = l.accept().unwrap();
            let r1 = super::is_ws_upgrade(&s1);
            let (s2, _) = l.accept().unwrap();
            let r2 = super::is_ws_upgrade(&s2);
            let (s3, _) = l.accept().unwrap();
            let r3 = super::is_ws_upgrade(&s3);
            (r1, r2, r3)
        });
        let mut c1 = TcpStream::connect(addr).unwrap();
        c1.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut c2 = TcpStream::connect(addr).unwrap();
        c2.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        // 分包握手：前半不含 Upgrade 行先到，~80ms 后剩余（含 Upgrade 行 + 终止符）才到——
        // 单发 peek 会在前半到达时就误判成 HTTP；有界重试要等到终止符或超时才下判断。
        let mut c3 = TcpStream::connect(addr).unwrap();
        c3.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(80));
        c3.write_all(b"Upgrade: websocket\r\n\r\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let (r1, r2, r3) = t.join().unwrap();
        assert!(!r1, "普通 GET 不是升级");
        assert!(r2, "Upgrade 头应判 WS");
        assert!(r3, "分包到达的 Upgrade 头仍应判 WS（不误判成 HTTP）");
    }

    #[test]
    fn http_path_serves_embedded_page() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            super::handle_conn(s, 1).unwrap();
        });
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        c.read_to_string(&mut resp).unwrap();
        t.join().unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("<canvas"), "应回内嵌页面");
        assert!(resp.contains("stg-engine viewer"));
    }
}
