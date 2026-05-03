/// Combined diff renderer: merges Bicep what-if output with rigg plan diffs
/// into a single human-readable summary.
///
/// Entry point: [`render`].
use colored::Colorize;

use crate::azure::deploy::whatif::WhatIfReport;
use crate::azure::rigg::plan::{PlanReport as RiggPlan, ResourceRef};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render a combined diff of Bicep infrastructure changes and rigg resource
/// configuration changes.
///
/// The output is a human-readable string suitable for printing to a terminal.
/// Color codes are emitted when the terminal supports them
/// (`colored::control::SHOULD_COLORIZE`).
///
/// The returned string does **not** include a trailing `[y/N]` prompt — that
/// is the caller's responsibility.
pub fn render(bicep: &WhatIfReport, rigg: &RiggPlan) -> String {
    let bicep_section = render_bicep_section(bicep);
    let rigg_section = render_rigg_section(rigg);

    let bicep_change_count = bicep.creates.len() + bicep.modifies.len() + bicep.deletes.len();
    let rigg_change_count = rigg.creates.len() + rigg.updates.len() + rigg.deletes.len();

    let summary = render_summary(bicep_change_count, rigg_change_count);

    if bicep_change_count == 0 && rigg_change_count == 0 {
        return "No changes pending.\n".to_string();
    }

    format!(
        "\
{sep}
Bicep (resource shells):
{bicep_section}
rigg (resource configuration):
{rigg_section}
{summary}",
        sep = separator(),
        bicep_section = bicep_section,
        rigg_section = rigg_section,
        summary = summary,
    )
}

// ---------------------------------------------------------------------------
// Bicep section
// ---------------------------------------------------------------------------

fn render_bicep_section(bicep: &WhatIfReport) -> String {
    let mut lines: Vec<String> = Vec::new();

    for c in &bicep.creates {
        let label = format!("{}/{}", c.resource_type, c.resource_id);
        lines.push(format!("{}", format!("+ {:<60} (Create)", label).green()));
    }

    for m in &bicep.modifies {
        let label = format!("{}/{}", m.resource_type, m.resource_id);
        lines.push(format!("{}", format!("~ {:<60}", label).yellow()));
        for fc in &m.field_changes {
            let from_str = json_val_short(&fc.from);
            let to_str = json_val_short(&fc.to);
            lines.push(format!(
                "    {}: {}",
                fc.path,
                format!("{from_str} → {to_str}").yellow()
            ));
        }
    }

    for d in &bicep.deletes {
        let label = format!("{}/{}", d.resource_type, d.resource_id);
        lines.push(format!("{}", format!("- {:<60} (Delete)", label).red()));
    }

    for u in &bicep.unchanged {
        let label = format!("{}/{}", u.resource_type, u.resource_id);
        lines.push(format!(
            "{}",
            format!("= {:<60} (Unchanged)", label).dimmed()
        ));
    }

    if lines.is_empty() {
        lines.push("  (no Bicep changes)".to_string());
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// rigg section
// ---------------------------------------------------------------------------

fn render_rigg_section(rigg: &RiggPlan) -> String {
    let mut lines: Vec<String> = Vec::new();

    for r in &rigg.creates {
        let label = resource_label(r);
        lines.push(format!("{}", format!("+ {:<60} (Create)", label).green()));
    }

    for (r, diff) in &rigg.updates {
        let label = resource_label(r);
        lines.push(format!("{}", format!("~ {:<60}", label).yellow()));
        for fc in &diff.field_changes {
            let from_str = json_val_short(&fc.from);
            let to_str = json_val_short(&fc.to);
            lines.push(format!(
                "    {}: {}",
                fc.path,
                format!("{from_str} → {to_str}").yellow()
            ));
        }
    }

    for r in &rigg.deletes {
        let label = resource_label(r);
        lines.push(format!("{}", format!("- {:<60} (Delete)", label).red()));
    }

    for r in &rigg.unchanged {
        let label = resource_label(r);
        lines.push(format!(
            "{}",
            format!("= {:<60} (Unchanged)", label).dimmed()
        ));
    }

    if lines.is_empty() {
        lines.push("  (no rigg changes)".to_string());
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Summary line
// ---------------------------------------------------------------------------

fn render_summary(bicep_changes: usize, rigg_changes: usize) -> String {
    format!(
        "{} Bicep change{}, {} rigg change{} pending.",
        bicep_changes,
        plural(bicep_changes),
        rigg_changes,
        plural(rigg_changes),
    )
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn separator() -> String {
    "─".repeat(66)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn resource_label(r: &ResourceRef) -> String {
    format!("{:?}/{}", r.kind, r.name).to_lowercase()
}

/// Render a `serde_json::Value` compactly for the diff display.
fn json_val_short(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => format!("[{} items]", a.len()),
        serde_json::Value::Object(_) => "{...}".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::deploy::whatif::{FieldChange as WhatIfFieldChange, ResourceChange};
    use crate::azure::rigg::plan::{
        FieldChange as RiggFieldChange, PlanReport, ResourceDiff, ResourceRef,
    };
    use rigg_core::resources::ResourceKind;

    fn empty_bicep() -> WhatIfReport {
        WhatIfReport {
            creates: vec![],
            modifies: vec![],
            deletes: vec![],
            unchanged: vec![],
            raw_json: serde_json::Value::Null,
        }
    }

    fn empty_rigg() -> PlanReport {
        PlanReport::default()
    }

    // Force color off for snapshot tests so output is deterministic.
    fn with_no_color<F: FnOnce() -> String>(f: F) -> String {
        colored::control::set_override(false);
        let result = f();
        colored::control::unset_override();
        result
    }

    #[test]
    fn render_handles_empty_reports() {
        let output = with_no_color(|| render(&empty_bicep(), &empty_rigg()));
        assert_eq!(output.trim(), "No changes pending.");
    }

    #[test]
    fn render_shows_creates() {
        let mut bicep = empty_bicep();
        bicep.creates.push(ResourceChange {
            resource_type: "Microsoft.App/containerApps".to_string(),
            resource_id: "quelch-prod-mcp".to_string(),
            field_changes: vec![],
        });
        let output = with_no_color(|| render(&bicep, &empty_rigg()));
        assert!(
            output.contains("+ Microsoft.App/containerApps/quelch-prod-mcp"),
            "output should contain create line, got:\n{output}"
        );
        assert!(output.contains("(Create)"));
    }

    #[test]
    fn render_shows_modifies_with_field_changes() {
        let mut bicep = empty_bicep();
        bicep.modifies.push(ResourceChange {
            resource_type: "Microsoft.DocumentDB/databaseAccounts".to_string(),
            resource_id: "quelch-prod-cosmos".to_string(),
            field_changes: vec![WhatIfFieldChange {
                path: "properties.throughput.mode".to_string(),
                from: serde_json::json!("serverless"),
                to: serde_json::json!("provisioned"),
            }],
        });
        let output = with_no_color(|| render(&bicep, &empty_rigg()));
        assert!(
            output.contains("~ Microsoft.DocumentDB/databaseAccounts/quelch-prod-cosmos"),
            "output should contain modify line"
        );
        assert!(
            output.contains("serverless → provisioned"),
            "output should contain field diff"
        );
    }

    #[test]
    fn render_shows_rigg_creates() {
        let mut rigg = empty_rigg();
        rigg.creates.push(ResourceRef {
            kind: ResourceKind::Index,
            name: "jira-issues".to_string(),
        });
        let output = with_no_color(|| render(&empty_bicep(), &rigg));
        assert!(
            output.contains("+ index/jira-issues"),
            "output should contain rigg create line, got:\n{output}"
        );
    }

    #[test]
    fn render_summary_line_counts_correctly() {
        let mut bicep = empty_bicep();
        bicep.creates.push(ResourceChange {
            resource_type: "Microsoft.App/containerApps".to_string(),
            resource_id: "app1".to_string(),
            field_changes: vec![],
        });
        bicep.creates.push(ResourceChange {
            resource_type: "Microsoft.App/containerApps".to_string(),
            resource_id: "app2".to_string(),
            field_changes: vec![],
        });
        bicep.modifies.push(ResourceChange {
            resource_type: "Microsoft.DocumentDB/databaseAccounts".to_string(),
            resource_id: "cosmos1".to_string(),
            field_changes: vec![],
        });
        // 3 Bicep changes total.

        let mut rigg = empty_rigg();
        for i in 0..6 {
            rigg.creates.push(ResourceRef {
                kind: ResourceKind::Index,
                name: format!("index-{i}"),
            });
        }
        // 6 rigg changes.

        let output = with_no_color(|| render(&bicep, &rigg));
        assert!(
            output.contains("3 Bicep changes"),
            "summary should say '3 Bicep changes', got:\n{output}"
        );
        assert!(
            output.contains("6 rigg changes"),
            "summary should say '6 rigg changes', got:\n{output}"
        );
    }

    #[test]
    fn render_full_snapshot() {
        // Deterministic snapshot of the combined diff view.
        let mut bicep = empty_bicep();
        bicep.creates.push(ResourceChange {
            resource_type: "Microsoft.App/containerApps".to_string(),
            resource_id: "quelch-prod-mcp".to_string(),
            field_changes: vec![],
        });
        bicep.modifies.push(ResourceChange {
            resource_type: "Microsoft.DocumentDB/databaseAccounts".to_string(),
            resource_id: "quelch-prod-cosmos".to_string(),
            field_changes: vec![WhatIfFieldChange {
                path: "properties.throughput.mode".to_string(),
                from: serde_json::json!("serverless"),
                to: serde_json::json!("provisioned"),
            }],
        });
        bicep.unchanged.push(ResourceChange {
            resource_type: "Microsoft.Search/searchServices".to_string(),
            resource_id: "quelch-prod-search".to_string(),
            field_changes: vec![],
        });

        let mut rigg = empty_rigg();
        rigg.creates.push(ResourceRef {
            kind: ResourceKind::Index,
            name: "jira-issues".to_string(),
        });
        rigg.creates.push(ResourceRef {
            kind: ResourceKind::Skillset,
            name: "jira-issues-vectorise".to_string(),
        });
        rigg.creates.push(ResourceRef {
            kind: ResourceKind::Indexer,
            name: "jira-issues".to_string(),
        });
        rigg.creates.push(ResourceRef {
            kind: ResourceKind::KnowledgeSource,
            name: "jira-issues".to_string(),
        });
        rigg.creates.push(ResourceRef {
            kind: ResourceKind::KnowledgeBase,
            name: "quelch-prod-kb".to_string(),
        });
        rigg.updates.push((
            ResourceRef {
                kind: ResourceKind::Index,
                name: "confluence-pages".to_string(),
            },
            ResourceDiff {
                field_changes: vec![RiggFieldChange {
                    path: "fields.component_path".to_string(),
                    from: serde_json::Value::Null,
                    to: serde_json::json!("Edm.String"),
                }],
            },
        ));

        let output = with_no_color(|| render(&bicep, &rigg));
        insta::assert_snapshot!(output);
    }
}
