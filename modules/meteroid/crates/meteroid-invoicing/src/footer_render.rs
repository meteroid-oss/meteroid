use base64::Engine;
use base64::engine::general_purpose::STANDARD as Base64Engine;
use maud::{DOCTYPE, Markup, html};

static CSS: &str = include_str!("../assets/footer.css");
static METEROID_FOOTER_LOGO: &[u8] = include_bytes!("../assets/footer-logo.png");

// we render the footer separately as required by gotenberg, to be rendered on each page. Global css & assets are not supported so it must be self-contained
pub(crate) fn render_footer() -> Markup {
    let image_base64 = Base64Engine.encode(METEROID_FOOTER_LOGO);
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="UTF-8";
                meta name="viewport" content="width=device-width, initial-scale=1.0";
                style { (CSS) }
            }
            body class="footer" {
                p class="padding" {}
                p class="poweredBy" {
                    img src=("data:image/png;base64,".to_owned() + &image_base64) alt="Meteroid" class="logo";
                    " | Powered by "
                    // links are currently not kept in conversion from PDF to PDF-3A (required for e-invoicing) cf: https://github.com/gotenberg/gotenberg/issues/972 :wontfix by gotenberg
                    // We could look into alternatives for the conversion step
                    // a href="https://meteroid.com" { "Meteroid.com" }
                    " Meteroid.com"
                    " - Billing solutions for sustainable growth"
                }
                p class="flex-1" {}
                p class="page" {
                    span class="pageNumber" {}
                    " / "
                    span class="totalPages" {}
                }
                p class="padding" {}


            }
        }
    }
}
