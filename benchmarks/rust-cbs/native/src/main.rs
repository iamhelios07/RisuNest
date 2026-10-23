mod arithmetic;
pub mod cbs;
use cbs::{compile, evaluate, Budget, Context, Units};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::time::Instant;
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireContext {
    #[serde(default)]
    variables: Vec<(Units, Units)>,
    #[serde(default)]
    globals: Vec<(Units, Units)>,
    #[serde(default)]
    index: i64,
    #[serde(default)]
    user: Units,
    #[serde(default)]
    seed: u32,
}
impl From<WireContext> for Context {
    fn from(w: WireContext) -> Self {
        Self {
            variables: w.variables.into_iter().collect(),
            globals: w.globals.into_iter().collect(),
            index: w.index,
            user: w.user,
            state: w.seed,
            ..Default::default()
        }
    }
}
fn context(v: &Value) -> Result<Context, String> {
    let w: WireContext = serde_json::from_value(v.clone()).map_err(|_| "context")?;
    if w.variables.len() + w.globals.len() > 4096
        || w.variables
            .iter()
            .chain(&w.globals)
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
            + w.user.len()
            > 1_048_576
    {
        return Err("context-limit".into());
    }
    Ok(w.into())
}
fn text(v: &Value) -> Result<Units, String> {
    if let Some(s) = v.as_str() {
        Ok(cbs::units(s))
    } else {
        serde_json::from_value(v.clone()).map_err(|_| "text".into())
    }
}
fn main() {
    let mut programs: HashMap<String, (Vec<cbs::Node>, usize)> = HashMap::new();
    let mut ctx = Context::default();
    let mut revision = 0u64;
    let mut bytes = 0usize;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let start = Instant::now();
        let result = (|| -> Result<Value, String> {
            let req: Value = serde_json::from_str(&line).map_err(|_| "request")?;
            let op = req["op"].as_str().ok_or("op")?;
            match op {
                "register" => {
                    let id = req["id"].as_str().ok_or("id")?.to_owned();
                    let source = text(&req["text"])?;
                    let mut p = compile(&source)?;
                    let folded = if req["fold"] == true {
                        cbs::fold_constants(&mut p)
                    } else {
                        0
                    };
                    let size = cbs::retained_bytes(&p);
                    let old = programs.get(&id).map(|(_, n)| *n).unwrap_or(0);
                    if programs.len() >= 64 && !programs.contains_key(&id)
                        || bytes - old + size > 8_388_608
                    {
                        return Err("resident-limit".into());
                    }
                    programs.insert(id, (p, size));
                    bytes = bytes - old + size;
                    Ok(json!({"programs":programs.len(),"retainedBytes":bytes,"folded":folded}))
                }
                "context" => {
                    ctx = context(&req["context"])?;
                    revision += 1;
                    Ok(json!({"revision":revision}))
                }
                "eval" | "evaluate" => {
                    if let Some(expected) = req["revision"].as_u64() {
                        if expected != revision {
                            return Err("stale-context".into());
                        }
                    }
                    let mut current = if req["context"].is_object() {
                        context(&req["context"])?
                    } else {
                        ctx.clone()
                    };
                    current.temp.clear();
                    current.draws = 0;
                    let mut budget = Budget::default();
                    if let Some(n) = req["budget"].as_u64() {
                        budget.remaining = n as usize
                    }
                    if req["cancelled"] == true {
                        budget
                            .cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed)
                    }
                    let local;
                    let p = if let Some(id) = req["id"].as_str() {
                        &programs.get(id).ok_or("missing-program")?.0
                    } else {
                        local = {
                            let mut nodes = compile(&text(&req["text"])?)?;
                            if req["fold"] == true {
                                cbs::fold_constants(&mut nodes);
                            }
                            nodes
                        };
                        &local
                    };
                    let count = req["repeat"].as_u64().unwrap_or(1).min(10_000);
                    let initial = current.clone();
                    let mut out = Vec::new();
                    for _ in 0..count {
                        current = initial.clone();
                        out = evaluate(p, &mut current, &mut budget)?
                    }
                    let encoded = match String::from_utf16(&out) {
                        Ok(s) => json!(s),
                        Err(_) => json!(out),
                    };
                    Ok(
                        json!({"output":encoded,"draws":current.draws,"rngState":current.state,"revision":revision,"retainedBytes":bytes,"programs":programs.len()}),
                    )
                }
                "release" => {
                    programs.clear();
                    programs.shrink_to_fit();
                    ctx = Context::default();
                    bytes = 0;
                    revision += 1;
                    Ok(json!({"programs":0,"retainedBytes":0,"variables":0,"revision":revision}))
                }
                "stats" => Ok(
                    json!({"programs":programs.len(),"retainedBytes":bytes,"variables":ctx.variables.len(),"revision":revision}),
                ),
                _ => Err("op".into()),
            }
        })();
        let mut result = match result {
            Ok(v) => v,
            Err(e) => json!({"fallback":e}),
        };
        result["coreMs"] = json!(start.elapsed().as_secs_f64() * 1000.0);
        if writeln!(output, "{}", result).is_err() {
            break;
        }
        if output.flush().is_err() {
            break;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_is_cooperative_and_discards_local_state() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = flag.clone();
        let handle = std::thread::spawn(move || {
            let program =
                vec![cbs::Node::Macro(vec![cbs::Node::Text(cbs::units("random::a::b"))]); 200_000];
            let mut c = Context::default();
            let mut b = Budget {
                cancel: flag,
                ..Budget::default()
            };
            evaluate(&program, &mut c, &mut b)
        });
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(handle.join().unwrap().unwrap_err(), "cancelled");
    }
    #[test]
    fn cancellation_of_a_running_evaluator() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = flag.clone();
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let child_progress = progress.clone();
        let handle = std::thread::spawn(move || {
            let p = vec![cbs::Node::Text(vec![65]); 400_000];
            let mut c = Context::default();
            let mut b = Budget {
                cancel: flag,
                progress: child_progress,
                ..Budget::default()
            };
            evaluate(&p, &mut c, &mut b)
        });
        while progress.load(std::sync::atomic::Ordering::Relaxed) < 100 {
            std::thread::yield_now()
        }
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(handle.join().unwrap().unwrap_err(), "cancelled");
    }
}
