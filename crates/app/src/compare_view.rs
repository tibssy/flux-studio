//! GPU-backed before/after split view.
//!
//! The compare overlay used to rasterise two full-resolution Cairo surfaces on
//! every pointer move. This widget keeps the images as `GdkTexture`s and only
//! records two texture nodes with a clip, so dragging the divider stays cheap
//! even for large upscales.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct CompareView {
        pub after: RefCell<Option<gdk::Texture>>,
        pub before: RefCell<Option<gdk::Texture>>,
        pub split: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompareView {
        const NAME: &'static str = "ImagiflumeCompareView";
        type Type = super::CompareView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for CompareView {}

    impl WidgetImpl for CompareView {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (1, 1, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let width = self.obj().width() as f32;
            let height = self.obj().height() as f32;
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            let after = self.after.borrow();
            let before = self.before.borrow();
            let (Some(after), Some(before)) = (after.as_ref(), before.as_ref()) else {
                return;
            };

            // The current image spans the canvas; the source replaces the
            // left side so the split reads as before/after.
            draw_contained(snapshot, after, width, height);
            let split = (self.split.get() as f32).clamp(0.0, 1.0) * width;
            snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, split, height));
            draw_contained(snapshot, before, width, height);
            snapshot.pop();

            snapshot.append_color(
                &gdk::RGBA::new(1.0, 1.0, 1.0, 0.9),
                &graphene::Rect::new(split - 1.0, 0.0, 2.0, height),
            );
            let knob = graphene::Rect::new(split - 12.0, height / 2.0 - 12.0, 24.0, 24.0);
            let radius = graphene::Size::new(12.0, 12.0);
            let outline = gsk::RoundedRect::new(knob, radius, radius, radius, radius);
            snapshot.push_rounded_clip(&outline);
            snapshot.append_color(&gdk::RGBA::new(0.05, 0.05, 0.05, 0.55), &knob);
            snapshot.pop();

            let inner = graphene::Rect::new(split - 9.0, height / 2.0 - 9.0, 18.0, 18.0);
            let inner_radius = graphene::Size::new(9.0, 9.0);
            let inner_outline = gsk::RoundedRect::new(
                inner,
                inner_radius,
                inner_radius,
                inner_radius,
                inner_radius,
            );
            snapshot.push_rounded_clip(&inner_outline);
            snapshot.append_color(&gdk::RGBA::new(1.0, 1.0, 1.0, 0.9), &inner);
            snapshot.pop();
        }
    }

    fn draw_contained(snapshot: &gtk::Snapshot, texture: &gdk::Texture, width: f32, height: f32) {
        let texture_width = texture.width() as f32;
        let texture_height = texture.height() as f32;
        if texture_width <= 0.0 || texture_height <= 0.0 {
            return;
        }
        let scale = (width / texture_width).min(height / texture_height);
        let scaled_width = texture_width * scale;
        let scaled_height = texture_height * scale;
        snapshot.append_texture(
            texture,
            &graphene::Rect::new(
                (width - scaled_width) / 2.0,
                (height - scaled_height) / 2.0,
                scaled_width,
                scaled_height,
            ),
        );
    }
}

glib::wrapper! {
    pub struct CompareView(ObjectSubclass<imp::CompareView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for CompareView {
    fn default() -> Self {
        Self::new()
    }
}

impl CompareView {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_images(&self, after: gdk::Texture, before: gdk::Texture) {
        *self.imp().after.borrow_mut() = Some(after);
        *self.imp().before.borrow_mut() = Some(before);
        self.queue_draw();
    }

    pub fn clear(&self) {
        self.imp().after.borrow_mut().take();
        self.imp().before.borrow_mut().take();
        self.queue_draw();
    }

    pub fn set_split(&self, split: f64) {
        self.imp().split.set(split.clamp(0.0, 1.0));
        self.queue_draw();
    }

    pub fn split(&self) -> f64 {
        self.imp().split.get()
    }
}
