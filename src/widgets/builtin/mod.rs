//! Built-in bottombar widgets.

pub mod aws;
pub mod command;
pub mod git;
pub mod kube;
pub mod time;

use crate::config::WidgetSpec;
use crate::widgets::Widget;

/// Build a built-in widget from its config spec. Unknown specs return
/// `None` so the rest of the bar still renders.
pub fn build(spec: &WidgetSpec) -> Option<Box<dyn Widget>> {
    Some(match spec {
        WidgetSpec::Git { align, interval_ms } => Box::new(git::GitWidget::new(
            align.unwrap_or(crate::config::AlignSpec::Left).into(),
            *interval_ms,
        )),
        WidgetSpec::Time { format, align, interval_ms } => Box::new(time::TimeWidget::new(
            format.clone().unwrap_or_else(|| "%H:%M:%S".to_string()),
            align.unwrap_or(crate::config::AlignSpec::Right).into(),
            *interval_ms,
        )),
        WidgetSpec::Kube { align, interval_ms } => Box::new(kube::KubeWidget::new(
            align.unwrap_or(crate::config::AlignSpec::Left).into(),
            *interval_ms,
        )),
        WidgetSpec::Aws { align, interval_ms } => Box::new(aws::AwsWidget::new(
            align.unwrap_or(crate::config::AlignSpec::Left).into(),
            *interval_ms,
        )),
        WidgetSpec::Command { name, command, on_click, align, interval_ms } => {
            Box::new(command::CommandWidget::new(
                name.clone(),
                command.clone(),
                on_click.clone(),
                align.unwrap_or(crate::config::AlignSpec::Left).into(),
                *interval_ms,
            ))
        }
    })
}
