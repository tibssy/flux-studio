//! A single-child container that clips its child to a rounded rectangle.
//!
//! The canvas picture keeps rounded corners through `RoundedPaintable`, but a
//! zoomed picture is clipped by the scroller at the viewport edge. Wrapping the
//! scroller in this widget rounds that edge too, so zooming never introduces
//! square corners.

use gtk::glib;
use gtk::graphene;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Matches `CORNER_RADIUS` in `rounded_paintable`.
const CORNER_RADIUS: f32 = 16.0;

glib::wrapper! {
    pub(crate) struct RoundedClip(ObjectSubclass<imp::RoundedClip>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RoundedClip {
    pub(crate) fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let clip: Self = glib::Object::builder().build();
        child.as_ref().set_parent(&clip);
        clip
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub(crate) struct RoundedClip;

    #[glib::object_subclass]
    impl ObjectSubclass for RoundedClip {
        const NAME: &'static str = "ImagiflumeRoundedClip";
        type Type = super::RoundedClip;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Accessible, gtk::Buildable);
    }

    impl ObjectImpl for RoundedClip {
        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_overflow(gtk::Overflow::Hidden);
            widget.set_layout_manager(Some(gtk::BinLayout::new()));
        }
    }

    impl WidgetImpl for RoundedClip {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let Some(child) = widget.first_child() else {
                return;
            };
            let bounds =
                graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, CORNER_RADIUS));
            widget.snapshot_child(&child, snapshot);
            snapshot.pop();
        }
    }

    impl AccessibleImpl for RoundedClip {}

    impl BuildableImpl for RoundedClip {}
}
