use iced::widget::{column, text};
use iced::Element;

#[derive(Debug, Clone)]
enum Message {}

#[derive(Default)]
struct App;

fn update(_app: &mut App, _m: Message) {}

fn view(_app: &App) -> Element<'_, Message> {
    column![text("hyprdisplays — hello").size(32)].padding(20).into()
}

fn main() -> iced::Result {
    iced::application(App::default, update, view)
        .title("hyprdisplays")
        .run()
}
