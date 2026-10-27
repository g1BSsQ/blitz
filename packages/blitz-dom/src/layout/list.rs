use blitz_traits::node_id::NodeId;
use markup5ever::local_name;
use parley::FontFamily;
use style::computed_values::list_style_type::T as ListStyleType;
use style::{
    computed_values::list_style_position::T as ListStylePosition,
    counter_style::CounterStyle,
    values::{computed::Contain, specified::box_::DisplayInside},
};
use stylo_taffy::CopyStyleGetters;

use crate::{
    BaseDocument,
    node::{ListItemLayout, ListItemLayoutPosition, Marker},
    stylo_to_parley,
};

pub(super) const BULLET_FONT_FAMILY: &str = "Bullet, monospace, sans-serif";

// Counter instances live across following siblings, but instances created by
// descendants are discarded when leaving their parent. The depth distinguishes
// an inherited counter from a counter reset by an earlier sibling.
#[derive(Clone, Copy)]
struct ListCounter {
    value: i32,
    reversed: bool,
    depth: usize,
}

#[derive(Default)]
struct CounterProperties {
    reset: Option<(i32, bool)>,
    increment: Option<i32>,
    value: Option<i32>,
    is_list_item: bool,
}

// Read the cascaded counter properties, not the CSS source text.
fn counter_properties(doc: &BaseDocument, node_id: NodeId) -> Option<CounterProperties> {
    let node = &doc.nodes[node_id];
    if node
        .display_style()
        .is_some_and(|display| display.inside() == DisplayInside::None)
    {
        return None;
    }

    let mut properties = CounterProperties::default();
    if let Some(style) = node.primary_styles() {
        let counters = style.get_counters();
        let list_item = |name: &style::values::CustomIdent| &*name.0 == "list-item";

        properties.reset = counters
            .counter_reset
            .iter()
            .rfind(|pair| list_item(&pair.name))
            .map(|pair| (pair.value, pair.is_reversed));
        for pair in counters.counter_increment.iter().filter(|pair| list_item(&pair.name)) {
            properties.increment = Some(
                properties
                    .increment
                    .unwrap_or(0)
                    .saturating_add(pair.value),
            );
        }
        properties.is_list_item = style.get_box().display.is_list_item();
        if let Some(el) = node.element_data() {
            if el.name.local == local_name!("ol") || el.name.local == local_name!("li") {
                eprintln!("DBGUB1 node={} tag={} reset={:?} inc={:?} reversed_attr={}", node_id, &*el.name.local, counters.counter_reset, counters.counter_increment, el.attr(local_name!("reversed")).is_some());
            }
        }
    }

    // HTML's value attribute sets the item's ordinal independent of CSS
    // counter-set, which the Servo Stylo build does not expose.
    if let Some(el) = node.element_data() {
        if el.name.local == local_name!("li") {
            properties.value = el.attr_parsed(local_name!("value"));
        }
    }

    Some(properties)
}

// Style containment scopes increments and sets in descendants, but not on
// the containing element itself. display:contents does not establish it.
fn has_style_containment(doc: &BaseDocument, node_id: NodeId) -> bool {
    let node = &doc.nodes[node_id];
    if node.display_style().is_some_and(|display| {
        matches!(display.inside(), DisplayInside::None | DisplayInside::Contents)
    }) {
        return false;
    }
    node.primary_styles()
        .is_some_and(|style| style.clone_contain().contains(Contain::STYLE))
}

fn child_ids(doc: &BaseDocument, node_id: NodeId) -> Vec<NodeId> {
    let node = &doc.nodes[node_id];
    node.before()
        .into_iter()
        .chain(node.children.iter().copied())
        .chain(node.after())
        .collect()
}

// CSS Lists 3 §4.4.2: an implicit reversed reset counts negated increments
// until the first explicit HTML item value, then adds that value.
fn reversed_initial_value(doc: &BaseDocument, node_id: NodeId) -> i32 {
    fn count(
        doc: &BaseDocument,
        node_id: NodeId,
        total: &mut i32,
        last_increment: &mut i32,
    ) -> bool {
        for child in child_ids(doc, node_id) {
            let Some(props) = counter_properties(doc, child) else {
                continue;
            };
            // A reset shadows this counter for the reset element and the
            // following siblings at this level. Nested lists cannot count.
            if props.reset.is_some() {
                break;
            }
            let increment = props
                .increment
                .unwrap_or(if props.is_list_item { -1 } else { 0 })
                .saturating_neg();
            if increment != 0 {
                *last_increment = increment;
            }
            if let Some(value) = props.value {
                *total = total.saturating_add(value);
                return true;
            }
            *total = total.saturating_add(increment);
            if !has_style_containment(doc, child)
                && count(doc, child, total, last_increment)
            {
                return true;
            }
        }
        false
    }

    let mut total: i32 = 0;
    let mut last_increment: i32 = 0;
    count(doc, node_id, &mut total, &mut last_increment);
    total.saturating_add(last_increment)
}

pub(crate) fn collect_list_item_children(doc: &mut BaseDocument, root: NodeId) {
    fn visit(
        doc: &mut BaseDocument,
        node_id: NodeId,
        depth: usize,
        counters: &mut Vec<ListCounter>,
    ) {
        let Some(props) = counter_properties(doc, node_id) else {
            return;
        };

        if let Some((value, reversed)) = props.reset {
            // A reset on a sibling replaces that sibling's counter, while a
            // reset on a descendant instantiates a nested counter.
            while counters.last().is_some_and(|counter| counter.depth >= depth) {
                counters.pop();
            }
            let value = if reversed && value == i32::MIN {
                reversed_initial_value(doc, node_id)
            } else {
                value
            };
            counters.push(ListCounter {
                value,
                reversed,
                depth,
            });
        }

        if props.is_list_item || props.increment.is_some() || props.value.is_some() {
            if counters.is_empty() {
                counters.push(ListCounter {
                    value: 0,
                    reversed: false,
                    depth,
                });
            }
            let counter = counters.last_mut().unwrap();
            let increment = props.increment.unwrap_or(if props.is_list_item {
                if counter.reversed { -1 } else { 1 }
            } else {
                0
            });
            counter.value = counter.value.saturating_add(increment);
            if let Some(value) = props.value {
                counter.value = value;
            }
        }

        let ordinal = counters.last().map_or(0, |counter| counter.value);
        if props.is_list_item { eprintln!("DBGUB1 ordinal node={} value={} reset={:?} stack={}", node_id, ordinal, props.reset, counters.len()); }
        let layout = if props.is_list_item {
            node_list_item_child(doc, node_id, ordinal)
        } else {
            None
        };
        if let Some(element) = doc.nodes[node_id].element_data_mut() {
            element.list_item_data = layout.map(Box::new);
        }

        // A descendant's reset survives among its siblings, but not beyond
        // the enclosing element. Updates to inherited counters do survive.
        if has_style_containment(doc, node_id) {
            let mut contained = Vec::new();
            for child in child_ids(doc, node_id) {
                visit(doc, child, depth + 1, &mut contained);
            }
        } else {
            let count = counters.len();
            for child in child_ids(doc, node_id) {
                visit(doc, child, depth + 1, counters);
            }
            counters.truncate(count);
        }
    }

    visit(doc, root, 0, &mut Vec::new());
}

// Return the marker for an element whose computed display is list-item.
fn node_list_item_child(
    doc: &mut BaseDocument,
    child_id: NodeId,
    ordinal: i32,
) -> Option<ListItemLayout> {
    let node = &doc.nodes[child_id];
    let styles = node.primary_styles()?;
    let list_style_type = styles.get_list_style_type();
    let list_style_position = styles.clone_list_style_position();
    let marker = marker_for_style(list_style_type, ordinal)?;

    let position = match list_style_position {
        ListStylePosition::Inside => ListItemLayoutPosition::Inside,
        ListStylePosition::Outside => {
            let mut parley_style = stylo_to_parley::style(child_id, &styles);

            if let Some(font_family) = font_for_bullet_style(list_style_type) {
                parley_style.font_family = font_family;
            }

            let mut font_ctx = doc.font_ctx.lock().unwrap();
            let mut builder = doc.layout_ctx.tree_builder(
                &mut font_ctx,
                doc.viewport.scale(),
                true,
                &parley_style,
            );

            match &marker {
                Marker::Char(char) => {
                    let mut buf = [0u8; 4];
                    builder.push_text(char.encode_utf8(&mut buf));
                }
                Marker::String(str) => builder.push_text(str),
            };

            let mut layout = builder.build().0;
            let width = layout.calculate_content_widths().max;
            layout.break_all_lines(Some(width));

            ListItemLayoutPosition::Outside(Box::new(layout))
        }
    };

    Some(ListItemLayout { marker, position })
}

// Format the counter's actual value. Styles with a positive-only range fall
// back to decimal for zero and negative values.
fn marker_for_style(list_style_type: &ListStyleType, ordinal: i32) -> Option<Marker> {
    let decimal = || Marker::String(format!("{ordinal}. "));
    Some(match &list_style_type.0 {
        CounterStyle::None => return None,
        CounterStyle::Name(name) => match &*name.0 {
            "lower-alpha" | "upper-alpha" if ordinal > 0 => {
                let mut marker = String::new();
                build_alpha_marker((ordinal - 1) as usize, &mut marker);
                if &*name.0 == "upper-alpha" {
                    marker.make_ascii_uppercase();
                }
                Marker::String(format!("{marker}. "))
            }
            "lower-roman" | "upper-roman" if (1..=3999).contains(&ordinal) => {
                let mut marker = String::new();
                build_roman_marker(ordinal as usize, &mut marker);
                if &*name.0 == "upper-roman" {
                    marker.make_ascii_uppercase();
                }
                Marker::String(format!("{marker}. "))
            }
            "decimal" => decimal(),
            "lower-alpha" | "upper-alpha" | "lower-roman" | "upper-roman" => decimal(),
            "disc" => Marker::Char('•'),
            "circle" => Marker::Char('◦'),
            "square" => Marker::Char('▪'),
            "disclosure-open" => Marker::Char('▾'),
            "disclosure-closed" => Marker::Char('▸'),
            _ => Marker::Char('□'),
        },
        CounterStyle::String(atom_string) => Marker::String(atom_string.as_ref().to_string()),
        CounterStyle::Symbols { symbols, .. } => {
            let syms = &symbols.0;
            if syms.is_empty() {
                Marker::Char('•')
            } else if ordinal <= 0 {
                decimal()
            } else {
                let sym = &syms[(ordinal as usize - 1) % syms.len()];
                let marker_str = match sym {
                    style::counter_style::Symbol::String(s) => s.to_string(),
                    style::counter_style::Symbol::Ident(id) => id.0.to_string(),
                };
                Marker::String(marker_str)
            }
        }
    })
}

// Override the font to our specific bullet font when rendering bullets
fn font_for_bullet_style(list_style_type: &ListStyleType) -> Option<FontFamily<'static>> {
    if list_style_type.0.is_bullet() {
        return Some(BULLET_FONT_FAMILY.into());
    }

    None
}

const ALPHABET: [char; 26] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z',
];

// Construct alphanumeric marker from index, appending characters when index exceeds powers of 26
fn build_alpha_marker(index: usize, str: &mut String) {
    let rem = index % 26;
    let sym = ALPHABET[rem];
    str.insert(0, sym);
    let rest = (index - rem) as i64 / 26 - 1;
    if rest >= 0 {
        build_alpha_marker(rest as usize, str);
    }
}

fn build_roman_marker(mut value: usize, marker: &mut String) {
    for (amount, symbol) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while value >= amount {
            marker.push_str(symbol);
            value -= amount;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::node::Marker;
    use style::{
        Atom,
        counter_style::CounterStyle,
        values::{CustomIdent, computed::ListStyleType},
    };

    use super::marker_for_style;

    fn list_style(s: &str) -> ListStyleType {
        ListStyleType(CounterStyle::Name(CustomIdent(Atom::from(s))))
    }

    #[test]
    fn test_marker_for_disc() {
        let result = marker_for_style(&ListStyleType::disc(), 1);
        assert_eq!(result, Some(Marker::Char('•')));
    }

    #[test]
    fn test_marker_for_decimal() {
        let style = list_style("decimal");
        assert_eq!(
            marker_for_style(&style, 1),
            Some(Marker::String("1. ".to_string()))
        );
        assert_eq!(
            marker_for_style(&style, -2),
            Some(Marker::String("-2. ".to_string()))
        );
    }

    #[test]
    fn test_marker_for_lower_alpha() {
        let style = list_style("lower-alpha");
        for (ordinal, expected) in [(1, "a. "), (2, "b. "), (27, "aa. "), (28, "ab. ")] {
            assert_eq!(
                marker_for_style(&style, ordinal),
                Some(Marker::String(expected.to_string()))
            );
        }
        assert_eq!(
            marker_for_style(&style, 0),
            Some(Marker::String("0. ".to_string()))
        );
    }

    #[test]
    fn test_marker_for_upper_alpha() {
        let style = list_style("upper-alpha");
        for (ordinal, expected) in [(1, "A. "), (2, "B. "), (27, "AA. "), (28, "AB. ")] {
            assert_eq!(
                marker_for_style(&style, ordinal),
                Some(Marker::String(expected.to_string()))
            );
        }
    }

    #[test]
    fn test_marker_for_roman() {
        let style = list_style("lower-roman");
        assert_eq!(
            marker_for_style(&style, 4),
            Some(Marker::String("iv. ".to_string()))
        );
        let style = list_style("upper-roman");
        assert_eq!(
            marker_for_style(&style, 9),
            Some(Marker::String("IX. ".to_string()))
        );
        assert_eq!(
            marker_for_style(&style, -1),
            Some(Marker::String("-1. ".to_string()))
        );
    }
}
