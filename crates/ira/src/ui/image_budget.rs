use super::state::SharedState;

pub struct ImageLoadBudget {
    remaining: usize,
    deferred: Vec<(gtk4::Image, String)>,
}

impl ImageLoadBudget {
    pub fn new(budget: usize) -> Self {
        Self {
            remaining: budget,
            deferred: Vec::new(),
        }
    }

    pub fn load(&mut self, img: &gtk4::Image, path: &str) {
        if path.is_empty() {
            return;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
            ira_images::set_image_async(img, path);
        } else {
            self.deferred.push((img.clone(), path.to_string()));
        }
    }

    pub fn flush(self, state: &SharedState, gen: u32) {
        if self.deferred.is_empty() {
            return;
        }
        let reqs = self.deferred;
        let mut i = 0usize;
        let state = state.clone();
        glib::idle_add_local(move || {
            if state.borrow().view_generation != gen {
                return glib::ControlFlow::Break;
            }
            let end = (i + 12).min(reqs.len());
            for (img, path) in &reqs[i..end] {
                ira_images::set_image_async(img, path);
            }
            i = end;
            if i >= reqs.len() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}
