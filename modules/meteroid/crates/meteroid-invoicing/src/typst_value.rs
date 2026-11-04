/// Declares a struct and converts it into a typst dictionary keyed by its field names, both as
/// `into_dict()` and as `IntoValue`.
macro_rules! typst_dict_struct {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $($(#[$field_meta:meta])* $field_vis:vis $field:ident : $ty:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $($(#[$field_meta])* $field_vis $field: $ty),*
        }

        impl $name {
            #[must_use]
            pub fn into_dict(self) -> typst::foundations::Dict {
                typst::foundations::dict!($(stringify!($field) => self.$field),*)
            }
        }

        impl typst::foundations::IntoValue for $name {
            fn into_value(self) -> typst::foundations::Value {
                typst::foundations::Value::Dict(self.into_dict())
            }
        }
    };
}

pub(crate) use typst_dict_struct;
