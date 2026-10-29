use blitz_test_harness::Harness;

#[test]
fn definite_auto_table_distributes_excess_by_max_content() {
    let harness = Harness::from_html(
        "<table style='width:500px;table-layout:auto;border-spacing:0;font:16px monospace'>            <tr>                <td id='short' style='padding:0'>a</td>                <td id='long' style='padding:0'>aaaaaaaaaa</td>            </tr>        </table>",
    );

    let short = harness.layout_rect("#short").width;
    let long = harness.layout_rect("#long").width;
    assert!(
        long > short * 3.0,
        "max-content-weighted columns should have a large width difference: {short} vs {long}"
    );
}
