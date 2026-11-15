//! Deterministic selection of clipboard commands in one accumulated egui output batch.
//!
//! egui can run several layout passes for one input batch and appends the output of every
//! completed pass. Clipboard actions emitted by an earlier pass therefore survive a later pass
//! that emits no clipboard action. The integration applies this module to the merged command
//! list before doing any clipboard work.
//!
//! Policy: the last emitted `CopyText`/`CopyImage` wins as one regular-clipboard family, and the
//! last emitted `TextSelectionSettled` wins independently for PRIMARY. This is a batch policy,
//! not command-specific input provenance: a later programmatic command supersedes an earlier
//! input-triggered one. All other commands remain in their original order.

use egui::OutputCommand;

/// Which kind of regular-clipboard command won the batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegularClipboardKind {
    /// `OutputCommand::CopyText`.
    Text,
    /// `OutputCommand::CopyImage`.
    Image,
}

/// The winning regular-clipboard command and its position in the original command list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectedRegular {
    /// Index in [`egui::PlatformOutput::commands`].
    pub index: usize,
    /// Payload family of the command at [`Self::index`].
    pub kind: RegularClipboardKind,
}

/// Winning clipboard commands in one accumulated output batch.
///
/// A `None` index means that family had no command in this batch. The integration executes the
/// selected commands at their original positions and drops all superseded clipboard commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OutputSelection {
    /// Last regular-clipboard command (`CopyText` or `CopyImage`), if any.
    pub regular: Option<SelectedRegular>,

    /// Last PRIMARY (`TextSelectionSettled`) command, if any.
    pub primary: Option<usize>,
}

/// Select the last regular command and, independently, the last PRIMARY command.
///
/// This function only inspects commands. It does not encode images, clone text, consume an input
/// origin, or infer pass identities from `PlatformOutput::num_completed_passes`.
pub(crate) fn select_clipboard_commands(commands: &[OutputCommand]) -> OutputSelection {
    let mut selection = OutputSelection::default();

    for (index, command) in commands.iter().enumerate() {
        match command {
            OutputCommand::CopyText(_) => {
                selection.regular = Some(SelectedRegular {
                    index,
                    kind: RegularClipboardKind::Text,
                });
            }
            OutputCommand::CopyImage(_) => {
                selection.regular = Some(SelectedRegular {
                    index,
                    kind: RegularClipboardKind::Image,
                });
            }
            OutputCommand::TextSelectionSettled(_) => {
                selection.primary = Some(index);
            }
            OutputCommand::OpenUrl(_) => {}
        }
    }

    selection
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, ColorImage, Context, FontDefinitions, OpenUrl, OutputCommand, RawInput};

    fn ctx_with_two_passes() -> Context {
        let ctx = Context::default();
        ctx.set_fonts(FontDefinitions::empty());
        ctx.options_mut(|options| options.max_passes = 2.try_into().unwrap());
        ctx
    }

    fn image() -> ColorImage {
        ColorImage::new([1, 1], vec![Color32::RED])
    }

    fn selected_text(commands: &[OutputCommand], selection: OutputSelection) -> &str {
        let index = selection.regular.expect("a regular command wins").index;
        match &commands[index] {
            OutputCommand::CopyText(text) => text,
            other => panic!("expected CopyText at {index}, got {other:?}"),
        }
    }

    #[test]
    fn first_pass_only_copy_survives_a_later_pass_without_clipboard_output() {
        let ctx = ctx_with_two_passes();
        let mut pass = 0;
        let output = ctx.run_ui(RawInput::default(), |ui| {
            if pass == 0 {
                ui.ctx().copy_text("first pass".to_owned());
                ui.request_discard("exercise the second pass");
            }
            pass += 1;
        });
        assert_eq!(pass, 2);
        assert_eq!(output.platform_output.num_completed_passes, 2);

        let selection = select_clipboard_commands(&output.platform_output.commands);
        assert_eq!(
            selected_text(&output.platform_output.commands, selection),
            "first pass"
        );
        assert_eq!(selection.primary, None);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn duplicate_copy_in_both_passes_selects_the_final_command_once() {
        let ctx = ctx_with_two_passes();
        let mut pass = 0;
        let output = ctx.run_ui(RawInput::default(), |ui| {
            ui.ctx().copy_text(format!("pass {pass}"));
            if pass == 0 {
                ui.request_discard("exercise the second pass");
            }
            pass += 1;
        });
        assert_eq!(pass, 2);

        let selection = select_clipboard_commands(&output.platform_output.commands);
        assert_eq!(
            selected_text(&output.platform_output.commands, selection),
            "pass 1"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn last_regular_command_wins_across_text_and_image() {
        let ctx = ctx_with_two_passes();
        let mut pass = 0;
        let output = ctx.run_ui(RawInput::default(), |ui| {
            if pass == 0 {
                ui.ctx().copy_image(image());
                ui.ctx().copy_text("text after image".to_owned());
                ui.request_discard("exercise the second pass");
            }
            pass += 1;
        });
        assert_eq!(pass, 2);

        let selection = select_clipboard_commands(&output.platform_output.commands);
        assert_eq!(
            selection.regular,
            Some(SelectedRegular {
                index: 1,
                kind: RegularClipboardKind::Text,
            })
        );
        assert_eq!(
            selected_text(&output.platform_output.commands, selection),
            "text after image"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn image_after_text_wins_without_selecting_the_earlier_text() {
        let ctx = Context::default();
        ctx.set_fonts(FontDefinitions::empty());
        let output = ctx.run_ui(RawInput::default(), |ui| {
            ui.ctx().copy_text("superseded".to_owned());
            ui.ctx().copy_image(image());
        });

        let selection = select_clipboard_commands(&output.platform_output.commands);
        assert_eq!(
            selection.regular,
            Some(SelectedRegular {
                index: 1,
                kind: RegularClipboardKind::Image,
            })
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn winning_empty_text_is_an_explicit_selection_not_a_noop() {
        let ctx = Context::default();
        ctx.set_fonts(FontDefinitions::empty());
        let output = ctx.run_ui(RawInput::default(), |ui| {
            ui.ctx().copy_text("earlier".to_owned());
            ui.ctx().copy_text(String::new());
        });

        let selection = select_clipboard_commands(&output.platform_output.commands);
        assert_eq!(
            selection.regular.map(|regular| regular.kind),
            Some(RegularClipboardKind::Text)
        );
        assert_eq!(
            selected_text(&output.platform_output.commands, selection),
            ""
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn later_programmatic_copy_supersedes_a_first_pass_user_copy() {
        let ctx = ctx_with_two_passes();
        let mut pass = 0;
        let output = ctx.run_ui(RawInput::default(), |ui| {
            if pass == 0 {
                ui.ctx().copy_text("selected".to_owned());
                ui.request_discard("exercise the second pass");
            } else {
                ui.ctx().copy_text("status".to_owned());
            }
            pass += 1;
        });
        assert_eq!(pass, 2);

        let selection = select_clipboard_commands(&output.platform_output.commands);
        assert_eq!(
            selected_text(&output.platform_output.commands, selection),
            "status",
            "last emitted command wins regardless of pass or trigger"
        );
        output.drop_without_applying_deltas();
    }

    #[test]
    fn primary_is_independent_and_nonclipboard_order_is_preserved() {
        let ctx = Context::default();
        ctx.set_fonts(FontDefinitions::empty());
        let output = ctx.run_ui(RawInput::default(), |ui| {
            ui.ctx()
                .send_cmd(OutputCommand::TextSelectionSettled("first".to_owned()));
            ui.ctx()
                .open_url(OpenUrl::same_tab("https://example.com/one"));
            ui.ctx().copy_text("regular".to_owned());
            ui.ctx()
                .send_cmd(OutputCommand::TextSelectionSettled("second".to_owned()));
            ui.ctx()
                .open_url(OpenUrl::same_tab("https://example.com/two"));
        });

        let commands = &output.platform_output.commands;
        let selection = select_clipboard_commands(commands);
        assert_eq!(
            selection.primary,
            Some(3),
            "the last PRIMARY command wins independently"
        );
        assert_eq!(
            selected_text(commands, selection),
            "regular",
            "a later PRIMARY command does not supersede regular text"
        );
        assert!(matches!(commands[1], OutputCommand::OpenUrl(_)));
        assert!(matches!(commands[4], OutputCommand::OpenUrl(_)));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn two_primary_editors_select_the_last_emitted_selection() {
        let ctx = Context::default();
        ctx.set_fonts(FontDefinitions::empty());
        let output = ctx.run_ui(RawInput::default(), |ui| {
            // The command has no owner identity, so the deterministic policy is the later
            // emitted selection, not an attempt to infer which editor has focus.
            ui.ctx()
                .send_cmd(OutputCommand::TextSelectionSettled("editor A".to_owned()));
            ui.ctx()
                .send_cmd(OutputCommand::TextSelectionSettled("editor B".to_owned()));
        });

        let commands = &output.platform_output.commands;
        let selection = select_clipboard_commands(commands);
        assert_eq!(selection.primary, Some(1));
        assert!(matches!(
            &commands[1],
            OutputCommand::TextSelectionSettled(text) if text == "editor B"
        ));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn invalid_last_winner_remains_selected_without_resurrecting_an_earlier_copy() {
        let commands = vec![
            OutputCommand::CopyText("earlier".to_owned()),
            OutputCommand::CopyImage(image()),
            OutputCommand::OpenUrl(OpenUrl::same_tab("https://example.com")),
        ];

        let selection = select_clipboard_commands(&commands);
        assert_eq!(
            selection.regular,
            Some(SelectedRegular {
                index: 1,
                kind: RegularClipboardKind::Image,
            }),
            "validity/transport errors belong to the winner, never to an earlier command"
        );
    }

    #[test]
    fn separate_batches_are_not_deduplicated() {
        let ctx = Context::default();
        ctx.set_fonts(FontDefinitions::empty());

        for expected in ["same", "same"] {
            let output = ctx.run_ui(RawInput::default(), |ui| {
                ui.ctx().copy_text("same".to_owned());
            });
            let selection = select_clipboard_commands(&output.platform_output.commands);
            assert_eq!(
                selected_text(&output.platform_output.commands, selection),
                expected,
                "each logical batch keeps its own regular command"
            );
            output.drop_without_applying_deltas();
        }
    }
}
