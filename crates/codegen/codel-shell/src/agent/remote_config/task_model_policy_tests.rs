use super::*;

fn snapshot(families: &[Option<&str>]) -> TaskModelCatalogSnapshot {
    TaskModelCatalogSnapshot {
        eligible: families
            .iter()
            .enumerate()
            .map(|(index, family)| EligibleTaskModel {
                id: format!("model-{index}"),
                model_family: family.map(str::to_owned),
            })
            .collect(),
        authority: CatalogAuthority::Complete,
    }
}

#[test]
fn selection_is_hidden_only_for_an_enabled_all_codel_catalog() {
    let all_codel = || snapshot(&[Some("codel"), Some("codel")]);
    assert_eq!(
        TaskModelSelection::Inherited,
        resolve_presentation(true, all_codel()).selection
    );
    assert_eq!(
        TaskModelSelection::Selectable,
        resolve_presentation(true, snapshot(&[Some("codel"), None])).selection
    );
    assert_eq!(
        TaskModelSelection::Selectable,
        resolve_presentation(false, all_codel()).selection
    );
    let mut provisional = all_codel();
    provisional.authority = CatalogAuthority::Provisional;
    assert_eq!(
        TaskModelSelection::Selectable,
        resolve_presentation(true, provisional).selection
    );
}
