use blitz_dom::node::Marker;
use blitz_test_harness::Harness;

fn marker(harness: &Harness, selector: &str) -> Marker {
    harness
        .base()
        .get_node(harness.node(selector))
        .unwrap()
        .element_data()
        .unwrap()
        .list_item_data
        .as_ref()
        .unwrap()
        .marker
        .clone()
}

#[test]
fn list_ordinals_follow_start_value_and_computed_counter_set() {
    let harness = Harness::from_html(
        "<ol start='3'>            <li id='a'>first</li>            <li id='b' value='7'>second</li>            <li id='c'>third</li>            <li id='d' value='5' style='counter-set:list-item 20'>fourth</li>            <li id='e'>fifth</li>        </ol>",
    );

    for (selector, expected) in [
        ("#a", "3. "),
        ("#b", "7. "),
        ("#c", "8. "),
        ("#d", "20. "),
        ("#e", "21. "),
    ] {
        assert_eq!(marker(&harness, selector), Marker::String(expected.into()));
    }
}
