//! 多文件编译单元(整局流程刀 spec §1)。
//!
//! `CompileError` 没有 file 字段(ast.rs 契约出入,单文件世界的既定形态),本模块
//! 不改它,改"包装":① 每单元独立 parse 预检——语法错天然带局部行号、跨文件撞名
//! 在此层报(两处位置都知道);② 预检通过后按传入序拼接源文本走既有 `compile()`
//! 单管线(typeck/slots/codegen 零改动);③ 出错时把全局行号经边界表回译成
//! (文件名, 局部行号),src_line 已由管线从拼接文本回填、内容正确无需再动。
//! 产物不依赖收集顺序:codegen `build()` 按 sub 名排序出 canonical id(lib.rs)。

use super::{CompileError, compile, parse};
use stg_core::ecl::image::EclImage;

pub fn compile_units(units: &[(String, String)]) -> Result<EclImage, Vec<(String, CompileError)>> {
    if units.is_empty() {
        return Err(vec![(
            String::new(),
            CompileError {
                line: 0,
                col: 0,
                msg: "编译单元清单为空".to_string(),
                src_line: String::new(),
            },
        )]);
    }
    // ① 每单元独立 parse:语法错直接局部化;收集顶层名字做跨文件撞名预检。
    let mut errs: Vec<(String, CompileError)> = Vec::new();
    let mut seen: std::collections::BTreeMap<String, (usize, u32)> =
        std::collections::BTreeMap::new();
    for (i, (name, src)) in units.iter().enumerate() {
        match parse(src, name) {
            Ok(prog) => {
                let tops = prog
                    .subs
                    .iter()
                    .map(|s| (format!("sub '{}'", s.name), s.span.line))
                    .chain(
                        prog.consts
                            .iter()
                            .map(|c| (format!("const '{}'", c.name), c.span.line)),
                    )
                    .chain(
                        prog.xformdefs
                            .iter()
                            .map(|x| (format!("xformdef '{}'", x.name), x.span.line)),
                    );
                for (key, line) in tops {
                    if let Some((pi, pline)) = seen.get(&key) {
                        if *pi != i {
                            errs.push((
                                name.clone(),
                                CompileError {
                                    line,
                                    col: 1,
                                    msg: format!(
                                        "{key} 跨文件重复定义(另见 {}:{})",
                                        units[*pi].0, pline
                                    ),
                                    src_line: src
                                        .lines()
                                        .nth(line.saturating_sub(1) as usize)
                                        .unwrap_or("")
                                        .to_string(),
                                },
                            ));
                        } // 同文件重复留给既有 typeck/entryck,规则不双轨。
                    } else {
                        seen.insert(key, (i, line));
                    }
                }
            }
            Err(pe) => errs.extend(pe.into_iter().map(|e| (name.clone(), e))),
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    // ② 拼接(每单元保证换行收尾)+ 行号边界:bases[i] = 单元 i 之前累计的全局行数,
    //    即单元 i 内局部行 L 对应全局行 bases[i] + L。acc 必须数**拼接后 combined 里
    //    实际追加的物理换行数**——`src.lines().count()` 对空单元 `""` 返回 0,但拼接
    //    时因非 `\n` 收尾补了一个物理 `\n`(占 1 行),两者在空单元上不等价,会让 acc
    //    比实际物理行数少 1、后续单元的行号基准整体偏低(批审 batch-review-1.md
    //    Important-1)。改用"数 combined 里实际增加的换行符数"消除该角落:物理 `\n`
    //    个数 + 非换行收尾补的那一个(空串 `!ends_with('\n')` 为 true → 补 1,与拼接
    //    补的换行一致)。对"以 `\n` 收尾的正常文件"与旧式等值,只修正空/无尾换行角落。
    let mut combined = String::new();
    let mut bases: Vec<u32> = Vec::with_capacity(units.len());
    let mut acc: u32 = 0;
    for (_, src) in units {
        bases.push(acc);
        combined.push_str(src);
        if !src.ends_with('\n') {
            combined.push('\n');
        }
        acc += src.matches('\n').count() as u32 + u32::from(!src.ends_with('\n'));
    }
    // ③ 单管线编译 + 行号回译:全局行 L(1-based)属于满足 bases[i] < L <= bases[i]+行数
    //    的单元 i,即 bases 中最后一个 < L 的元素;局部行 = L - bases[i]。
    compile(&combined, "<multi>").map_err(|ce| {
        ce.into_iter()
            .map(|mut e| {
                let idx = bases.partition_point(|&b| b < e.line).saturating_sub(1);
                e.line -= bases[idx];
                (units[idx].0.clone(), e)
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(name: &str, src: &str) -> (String, String) {
        (name.to_string(), src.to_string())
    }

    #[test]
    fn cross_file_call_compiles_and_entry_resolves() {
        let img = compile_units(&[
            u("main.ecl", "sub main() { helper(); }\n"),
            u("lib.ecl", "sub helper() { wait(1); }\n"),
        ])
        .expect("跨文件同步调用必须编译通过");
        assert!(img.root().is_some(), "main 入口存在");
    }

    #[test]
    fn duplicate_sub_across_files_reports_both_positions() {
        let errs = compile_units(&[
            u("a.ecl", "sub main() { }\nsub foo() { }\n"),
            u("b.ecl", "sub foo() { }\n"),
        ])
        .expect_err("跨文件撞名必须报错");
        let (file, e) = &errs[0];
        assert_eq!(file, "b.ecl", "错误落在后出现的文件");
        assert!(
            e.msg.contains("foo") && e.msg.contains("a.ecl"),
            "消息含另一处位置:{}",
            e.msg
        );
    }

    #[test]
    fn parse_error_carries_local_line_and_file() {
        let errs = compile_units(&[
            u("ok.ecl", "sub main() { }\n"),
            // 缺分号:parser 在期待 ';' 处报错,实际下一个 token 是第 3 行的 '}'
            // （既有 parser 行为，探查核实：报错位置钉在“意外 token”而非语句自身行）。
            u("bad.ecl", "sub broken() {\n    wait(1)\n}\n"),
        ])
        .expect_err("语法错必须报");
        let (file, e) = &errs[0];
        assert_eq!(file, "bad.ecl");
        assert_eq!(e.line, 3, "行号必须是文件内局部行号,不是拼接后全局行号");
    }

    #[test]
    fn typeck_error_line_translates_back_to_unit() {
        // 类型错在第二个文件:拼接后全局行号必须回译成 lib.ecl 局部行号。
        let errs = compile_units(&[
            u("main.ecl", "sub main() { helper(); }\n"),
            u("lib.ecl", "sub helper() {\n    var x: fx = 1;\n}\n"), // int→fx 需 cast,第 2 行
        ])
        .expect_err("判型错必须报");
        let (file, e) = &errs[0];
        assert_eq!(file, "lib.ecl");
        assert_eq!(e.line, 2);
        assert!(
            !e.src_line.is_empty(),
            "src_line 已由管线回填且属于正确文件的文本"
        );
    }

    #[test]
    fn unit_order_does_not_change_image_bytes() {
        let a = [
            u("main.ecl", "sub main() { helper(); }\n"),
            u("lib.ecl", "sub helper() { wait(1); }\n"),
        ];
        let b = [a[1].clone(), a[0].clone()];
        let ia = compile_units(&a).expect("a 序");
        let ib = compile_units(&b).expect("b 序");
        assert_eq!(
            ia.code(),
            ib.code(),
            "canonical 按名排序 ⇒ 收集顺序不改产物"
        );
    }

    #[test]
    fn empty_units_rejected() {
        assert!(compile_units(&[]).is_err(), "空清单必须报错而非 panic");
    }

    #[test]
    fn empty_unit_does_not_shift_line_attribution() {
        // 注意:语法错(parse 阶段)在①预检就独立按单元 src 报,天然带正确局部行号,
        // 根本不经过②③的 acc/bases 拼接回译——不能拿它判别本缺陷。真正会经过
        // bases[] 回译的是③单管线 compile() 才能发现的错误(typeck/slots/codegen),
        // 故这里复用 typeck_error_line_translates_back_to_unit 的判型错场景。
        //
        // 对照:不含空单元时的期望行号。
        let baseline_errs = compile_units(&[
            u("main.ecl", "sub main() { helper(); }\n"),
            u("lib.ecl", "sub helper() {\n    var x: fx = 1;\n}\n"), // int→fx 需 cast,第 2 行
        ])
        .expect_err("判型错必须报");
        let (baseline_file, baseline_e) = &baseline_errs[0];
        assert_eq!(baseline_file, "lib.ecl");
        assert_eq!(baseline_e.line, 2, "无空单元时的期望行号(判别基准)");

        // 插入一个空单元(empty.ecl,0 字节)后,行号归属不得偏移、错误不得跑到别的文件。
        let errs = compile_units(&[
            u("main.ecl", "sub main() { helper(); }\n"),
            u("empty.ecl", ""),
            u("lib.ecl", "sub helper() {\n    var x: fx = 1;\n}\n"),
        ])
        .expect_err("判型错必须报");
        let (file, e) = &errs[0];
        assert_eq!(file, "lib.ecl", "空单元不得使错误归到别的文件");
        assert_eq!(
            e.line, baseline_e.line,
            "空单元不得使行号偏移(与无空单元时的行号一致)"
        );
    }
}
