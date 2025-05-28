use golden::golden;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
enum Message {
    Text { content: String },
    Image { url: String },
}

golden!(Message, {
    "text" => Message::Text { content: "Hello, world!".to_string() },
    "image" => Message::Image { url: "https://example.com/image.jpg".to_string() }
});
