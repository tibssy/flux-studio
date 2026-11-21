use std::cell::RefCell;

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

const CORNER_RADIUS: f32 = 16.0;

glib::wrapper! {
    pub(crate) struct RoundedPaintable(ObjectSubclass<imp::RoundedPaintable>)
        @implements gdk::Paintable;
}

impl RoundedPaintable {
    pub(crate) fn new(texture: &gdk::Texture) -> Self {
        let paintable: Self = glib::Object::builder().build();
        *paintable.imp().texture.borrow_mut() = Some(texture.clone());
        paintable
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub(crate) struct RoundedPaintable {
        pub(crate) texture: RefCell<Option<gdk::Texture>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoundedPaintable {
        const NAME: &'static str = "ImagiflumeRoundedPaintable";
        type Type = super::RoundedPaintable;
        type ParentType = glib::Object;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for RoundedPaintable {}

    impl PaintableImpl for RoundedPaintable {
        fn current_image(&self) -> gdk::Paintable {
            self.obj().clone().upcast()
        }

        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }

        fn intrinsic_width(&self) -> i32 {
            self.texture
                .borrow()
                .as_ref()
                .map_or(0, |texture| texture.width())
        }

        fn intrinsic_height(&self) -> i32 {
            self.texture
                .borrow()
                .as_ref()
                .map_or(0, |texture| texture.height())
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            let texture = self.texture.borrow();
            match texture.as_ref() {
                Some(texture) if texture.height() > 0 => {
                    texture.width() as f64 / texture.height() as f64
                }
                _ => 0.0,
            }
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let texture = self.texture.borrow();
            let Some(texture) = texture.as_ref() else {
                return;
            };
            let Some(snapshot) = snapshot.downcast_ref::<gtk::Snapshot>() else {
                texture.snapshot(snapshot, width, height);
                return;
            };
            let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, CORNER_RADIUS));
            texture.snapshot(snapshot, width, height);
            snapshot.pop();
        }
    }
}
