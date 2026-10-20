use super::*;
pub(super) fn layout(g: &mut Value, all: bool) -> Result<()> {
    let mut depth = HashMap::<String, usize>::new();
    let mut rows = HashMap::<usize, usize>::new();
    for id in order(g)? {
        let srcs = g["nodes"][&id]["inputs"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .map(refs)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let d = srcs
            .iter()
            .filter_map(|s| depth.get(s))
            .max()
            .map_or(0, |d| d + 1);
        depth.insert(id.clone(), d);
        let col = rows.entry(d).or_default();
        if all || g["nodes"][&id].get("pos").is_none() {
            g["nodes"][&id]["pos"] = json!([*col as f64 * 190., d as f64 * 110.]);
        }
        *col += 1;
    }
    Ok(())
}
pub(super) fn connect(
    g: &mut Value,
    src: &str,
    dst: &str,
    port: Option<&str>,
    index: Option<i64>,
) -> Result<String> {
    node(g, src)?;
    let s = spec(g, dst)?;
    let p = port
        .or_else(|| {
            s.inputs
                .iter()
                .find(|i| {
                    g["nodes"][dst]["inputs"]
                        .get(i.name)
                        .is_none_or(|v| v.is_null() || v.as_array().is_some_and(Vec::is_empty))
                })
                .map(|i| i.name)
        })
        .or_else(|| s.inputs.first().map(|i| i.name))
        .ok_or("node has no inputs")?;
    let i = s
        .inputs
        .iter()
        .find(|i| i.name == p)
        .ok_or_else(|| format!("node '{dst}' has no input '{p}'"))?;
    let v = if i.multi {
        let mut r = refs(&g["nodes"][dst]["inputs"][p])?;
        r.retain(|r| r != src);
        r.insert(
            index.unwrap_or(r.len() as i64).clamp(0, r.len() as i64) as usize,
            src.into(),
        );
        json!(r)
    } else {
        json!(src)
    };
    ensure_object(&mut g["nodes"][dst]["inputs"]).insert(p.into(), v);
    order(g)?;
    Ok(p.into())
}
pub(super) fn rewire(g: &mut Value, old: &str, new: Option<&str>) -> usize {
    let mut count = 0;
    for n in g["nodes"].as_object_mut().unwrap().values_mut() {
        if let Some(ins) = n.get_mut("inputs").and_then(Value::as_object_mut) {
            ins.retain(|_, v| {
                if v.as_str() == Some(old) {
                    count += 1;
                    if let Some(new) = new {
                        *v = json!(new);
                        true
                    } else {
                        false
                    }
                } else if let Some(a) = v.as_array_mut() {
                    a.retain_mut(|r| {
                        if r.as_str() == Some(old) {
                            count += 1;
                            if let Some(new) = new {
                                *r = json!(new);
                                true
                            } else {
                                false
                            }
                        } else {
                            true
                        }
                    });
                    !a.is_empty()
                } else {
                    true
                }
            });
        }
    }
    count
}
