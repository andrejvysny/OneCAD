//! Native asynchronous dialogs never block the presentation thread.
use onecad_app::host::{DialogPort, FileDialog};
use std::path::PathBuf;

pub struct NativeDialogs {
    directory: PathBuf,
}

impl NativeDialogs {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
}

#[async_trait::async_trait]
impl DialogPort for NativeDialogs {
    async fn pick(&self, request: FileDialog) -> Vec<String> {
        let dialog = rfd::AsyncFileDialog::new()
            .set_directory(&self.directory)
            .add_filter(&request.label, &request.extensions);
        let dialog = if let Some(filename) = &request.suggested_filename {
            dialog.set_file_name(filename)
        } else {
            dialog
        };
        let files = if request.save {
            dialog.save_file().await.into_iter().collect()
        } else if request.multiple {
            dialog.pick_files().await.unwrap_or_default()
        } else {
            dialog.pick_file().await.into_iter().collect()
        };
        files
            .into_iter()
            .map(|file| file.path().to_string_lossy().into_owned())
            .collect()
    }
}
