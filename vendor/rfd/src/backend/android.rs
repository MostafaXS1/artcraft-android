//! ArtCraft Android builder patch: a compile-only backend for `aarch64-linux-android`.
//!
//! Upstream rfd 0.17 has no Android backend, and its `FileDialog`/`MessageDialog` methods are typed
//! against these traits unconditionally, so rfd does not compile for Android at all. That breaks
//! desktop apps which call `rfd` directly (PDFCraft does, in several files) from being packaged for
//! Android.
//!
//! This module satisfies those trait bounds so the crate builds. The **async** dialogs and the
//! **synchronous** `FileDialog` dialogs all route through [`crate::artcraft_saf`], which the
//! generated/adapted Activity drives (the system Storage Access Framework pickers). When no
//! launcher is installed every dialog resolves to "cancelled" (no files, no message result), so an
//! app packaged this way launches and runs but its native file pickers are inert.

use super::{
    AsyncFilePickerDialogImpl, AsyncFileSaveDialogImpl, AsyncFolderPickerDialogImpl,
    AsyncMessageDialogImpl, DialogFutureType, FilePickerDialogImpl, FileSaveDialogImpl,
    FolderPickerDialogImpl, MessageDialogImpl,
};
use crate::file_dialog::FileDialog;
use crate::message_dialog::{MessageDialog, MessageDialogResult};
use crate::FileHandle;
use std::path::PathBuf;

impl FilePickerDialogImpl for FileDialog {
    fn pick_file(self) -> Option<PathBuf> {
        crate::artcraft_saf::pick_sync(crate::artcraft_saf::Kind::Open { multiple: false })
            .and_then(|paths| paths.into_iter().next())
    }

    fn pick_files(self) -> Option<Vec<PathBuf>> {
        crate::artcraft_saf::pick_sync(crate::artcraft_saf::Kind::Open { multiple: false })
    }
}

impl FileSaveDialogImpl for FileDialog {
    fn save_file(self) -> Option<PathBuf> {
        let name = self.file_name().to_owned();
        crate::artcraft_saf::pick_sync(crate::artcraft_saf::Kind::Save(name))
            .and_then(|paths| paths.into_iter().next())
    }
}

impl FolderPickerDialogImpl for FileDialog {
    fn pick_folder(self) -> Option<PathBuf> {
        crate::artcraft_saf::pick_sync(crate::artcraft_saf::Kind::Folder)
            .and_then(|paths| paths.into_iter().next())
    }

    fn pick_folders(self) -> Option<Vec<PathBuf>> {
        crate::artcraft_saf::pick_sync(crate::artcraft_saf::Kind::Folder)
    }
}

impl AsyncFilePickerDialogImpl for FileDialog {
    fn pick_file_async(self) -> DialogFutureType<Option<FileHandle>> {
        Box::pin(async move {
            crate::artcraft_saf::pick(crate::artcraft_saf::Kind::Open { multiple: false }).await
                .and_then(|paths| paths.into_iter().next())
                .map(FileHandle::wrap)
        })
    }

    fn pick_files_async(self) -> DialogFutureType<Option<Vec<FileHandle>>> {
        Box::pin(async move {
            crate::artcraft_saf::pick(crate::artcraft_saf::Kind::Open { multiple: true }).await
                .map(|paths| paths.into_iter().map(FileHandle::wrap).collect())
        })
    }
}

impl AsyncFolderPickerDialogImpl for FileDialog {
    fn pick_folder_async(self) -> DialogFutureType<Option<FileHandle>> {
        Box::pin(async move {
            crate::artcraft_saf::pick(crate::artcraft_saf::Kind::Folder).await
                .and_then(|paths| paths.into_iter().next())
                .map(FileHandle::wrap)
        })
    }

    fn pick_folders_async(self) -> DialogFutureType<Option<Vec<FileHandle>>> {
        Box::pin(async move {
            crate::artcraft_saf::pick(crate::artcraft_saf::Kind::Folder).await
                .map(|paths| paths.into_iter().map(FileHandle::wrap).collect())
        })
    }
}

impl AsyncFileSaveDialogImpl for FileDialog {
    fn save_file_async(self) -> DialogFutureType<Option<FileHandle>> {
        Box::pin(async move {
            let name = self.file_name().to_owned();
            crate::artcraft_saf::pick(crate::artcraft_saf::Kind::Save(name)).await
                .and_then(|paths| paths.into_iter().next())
                .map(FileHandle::wrap)
        })
    }
}

impl MessageDialogImpl for MessageDialog {
    fn show(self) -> MessageDialogResult {
        MessageDialogResult::Cancel
    }
}

impl AsyncMessageDialogImpl for MessageDialog {
    fn show_async(self) -> DialogFutureType<MessageDialogResult> {
        Box::pin(async { MessageDialogResult::Cancel })
    }
}
