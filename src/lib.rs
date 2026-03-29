use godot::prelude::*;

mod bridge;
mod fonts;
mod input;
mod rich_text_edit;
mod rich_text_view;

struct GodotRichText;

#[gdextension]
unsafe impl ExtensionLibrary for GodotRichText {}
