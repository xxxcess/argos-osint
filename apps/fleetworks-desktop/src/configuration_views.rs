//! Small native editors; secret inputs never appear in confirmation summaries.

use fleetworks_contracts::context::Permission;
use rune::El;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;
use rune_kit::Select;

use crate::app::Load;
use crate::app::State;
use crate::views::line;
use crate::views::quiet;

fn model_choice(
    state: &State,
    kit: &Kit,
    identity: (&str, &str),
    options: &[String],
    current: Option<&str>,
    choose: fn(&mut State, &str),
    layers: &mut Vec<El>,
) -> El {
    let (key, label) = identity;
    let picker = key.to_owned();
    Select::new(key, options)
        .label(label)
        .placeholder(label)
        .width(260.0)
        .current(current)
        .open(state.configuration.model_picker.as_deref() == Some(key))
        .disabled(options.is_empty() || !state.permission(Permission::ManageConfiguration))
        .on_toggle(move |state: &mut State| {
            state.configuration.model_picker =
                if state.configuration.model_picker.as_ref() == Some(&picker) {
                    None
                } else {
                    Some(picker.clone())
                };
        })
        .on_close(|state: &mut State| state.configuration.model_picker = None)
        .on_choose(move |state: &mut State, value, _| choose(state, value))
        .build(kit, layers)
}

/// Current backend choices only; account binding remains part of each route.
pub(crate) fn role_row_editor(state: &State, kit: &Kit, layers: &mut Vec<El>) -> El {
    let Load::Ready(defaults) = &state.backend.models else {
        return quiet("Model choices unavailable", kit);
    };
    let mut providers = defaults
        .catalog
        .iter()
        .map(|entry| entry.provider.clone())
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    let entries = defaults
        .catalog
        .iter()
        .filter(|entry| entry.provider == state.configuration.model_provider.text)
        .collect::<Vec<_>>();
    let models = entries
        .iter()
        .map(|entry| entry.label())
        .collect::<Vec<_>>();
    let selected = entries.iter().find(|entry| {
        entry.model == state.configuration.model.text
            && entry.account == state.configuration.model_account.text
    });
    let model_label = selected.map(|entry| entry.label());
    let key = format!(
        "{}|{}|{}",
        state.configuration.role.text,
        state.configuration.model_provider.text,
        state.configuration.model.text
    );
    let variant = match state.configuration.variant_updates.get(&key) {
        Some(value) => value.as_deref(),
        None => defaults.model_variants.get(&key).map(String::as_str),
    }
    .unwrap_or("Provider default");
    let mut efforts = vec!["Provider default".to_owned()];
    if let Some(entry) = selected {
        efforts.extend(entry.efforts.iter().cloned());
    }
    let fallbacks = defaults
        .catalog
        .iter()
        .map(|entry| format!("{} · {}", entry.provider, entry.label()))
        .collect::<Vec<_>>();
    let mut body = rune::col()
        .gap(8.0)
        .child(model_choice(
            state,
            kit,
            ("model-primary-provider", "Provider"),
            &providers,
            Some(&state.configuration.model_provider.text),
            State::select_model_provider,
            layers,
        ))
        .child(model_choice(
            state,
            kit,
            ("model-primary-model", "Model"),
            &models,
            model_label.as_deref(),
            State::select_catalog_model,
            layers,
        ))
        .child(quiet("Reasoning effort / variant", kit))
        .child(model_choice(
            state,
            kit,
            ("model-reasoning-variant", "Reasoning variant"),
            &efforts,
            Some(variant),
            State::select_model_variant,
            layers,
        ))
        .child(quiet("Ordered fallbacks", kit));
    if let Ok(routes) =
        serde_json::from_str::<Vec<crate::backend::ModelRoute>>(&state.configuration.fallbacks.text)
    {
        let count = routes.len();
        for (index, route) in routes.iter().enumerate() {
            body = body.child(
                rune::col()
                    .gap(4.0)
                    .child(line(format!("{}. {}", index + 1, route.label()), kit))
                    .child(
                        rune::row()
                            .gap(6.0)
                            .child(
                                Button::new("Move up")
                                    .variant(ButtonVariant::Ghost)
                                    .disabled(index == 0)
                                    .on_press(move |state: &mut State| {
                                        state.reorder_model_fallback(index, -1)
                                    })
                                    .build(kit)
                                    .key(format!("fallback-up-{index}")),
                            )
                            .child(
                                Button::new("Move down")
                                    .variant(ButtonVariant::Ghost)
                                    .disabled(index + 1 == count)
                                    .on_press(move |state: &mut State| {
                                        state.reorder_model_fallback(index, 1)
                                    })
                                    .build(kit)
                                    .key(format!("fallback-down-{index}")),
                            )
                            .child(
                                Button::new("Remove")
                                    .variant(ButtonVariant::Ghost)
                                    .on_press(move |state: &mut State| {
                                        state.reorder_model_fallback(index, 0)
                                    })
                                    .build(kit)
                                    .key(format!("fallback-remove-{index}")),
                            ),
                    ),
            );
        }
    }
    body.child(model_choice(
        state,
        kit,
        ("model-add-fallback", "Add fallback"),
        &fallbacks,
        None,
        State::add_model_fallback,
        layers,
    ))
    .child(
        Button::new("Review assignment")
            .variant(ButtonVariant::Outline)
            .disabled(
                !state.permission(Permission::ManageConfiguration)
                    || matches!(state.configuration.result, Load::Loading),
            )
            .on_press(|state: &mut State| state.preview_model_role(false))
            .build(kit)
            .key("model-preview"),
    )
}
