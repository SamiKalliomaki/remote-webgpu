use crate::{
    loader::{AssetLoader, ErasedAssetLoader},
    path::AssetPath,
};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use async_broadcast::RecvError;
use bevy_platform::collections::HashMap;
use bevy_tasks::IoTaskPool;
use bevy_utils::TypeIdMap;
use core::any::TypeId;
use thiserror::Error;
use tracing::warn;

#[derive(Default)]
pub(crate) struct AssetLoaders {
    loaders: Vec<MaybeAssetLoader>,
    type_id_to_loaders: TypeIdMap<Vec<usize>>,
    extension_to_loaders: HashMap<Box<str>, Vec<usize>>,
    type_path_to_loader: HashMap<&'static str, usize>,
    type_path_to_preregistered_loader: HashMap<&'static str, usize>,
}

impl AssetLoaders {
    /// The number of registered (or preregistered) loaders.
    pub(crate) fn len(&self) -> usize {
        self.loaders.len()
    }

    /// Get the [`AssetLoader`] stored at the specific index
    fn get_by_index(&self, index: usize) -> Option<MaybeAssetLoader> {
        self.loaders.get(index).cloned()
    }

    /// Registers a new [`AssetLoader`]. [`AssetLoader`]s must be registered before they can be used.
    pub(crate) fn push<L: AssetLoader>(&mut self, loader: L) {
        let type_path = L::type_path();
        // TODO: Allow using the short path of loaders.
        let loader_asset_type = TypeId::of::<L::Asset>();
        let loader_asset_type_name = core::any::type_name::<L::Asset>();

        let loader = Arc::new(loader);

        // FORK: registering a loader type that is already registered replaces
        // it in place instead of appending a second copy.  Upstream appends
        // unconditionally, which grows `loaders`, `type_id_to_loaders` and
        // `extension_to_loaders` for good and warns about "Duplicate
        // AssetLoader registered" -- and nothing can ever remove the entries.
        // That is a leak here, because every player who joins the game builds
        // a throwaway `App` (identical plugin stack, identical loaders) on the
        // one long-lived `AssetServer`.  A loader already filed under this
        // type path is by construction the same loader type, so the newest
        // instance can simply take over the slot; that also keeps the
        // one-loader-per-asset-type fast path in `find` working.  See
        // `docs/shared-asset-server-loader-growth.md`.
        //
        // A type path in `type_path_to_preregistered_loader` is a `reserve`d
        // slot still waiting for its loader, which is the case the `is_new ==
        // false` branch below exists to fill in; only an already-`Ready` slot
        // is a re-registration.
        if !self.type_path_to_preregistered_loader.contains_key(type_path)
            && let Some(&index) = self.type_path_to_loader.get(type_path)
        {
            self.loaders[index] = MaybeAssetLoader::Ready(loader);
            return;
        }

        let (loader_index, is_new) =
            if let Some(index) = self.type_path_to_preregistered_loader.remove(type_path) {
                (index, false)
            } else {
                (self.loaders.len(), true)
            };

        if is_new {
            let existing_loaders_for_type_id = self.type_id_to_loaders.get(&loader_asset_type);
            let mut duplicate_extensions = Vec::new();
            for extension in AssetLoader::extensions(&*loader) {
                let list = self
                    .extension_to_loaders
                    .entry((*extension).into())
                    .or_default();

                if !list.is_empty()
                    && let Some(existing_loaders_for_type_id) = existing_loaders_for_type_id
                    && list
                        .iter()
                        .any(|index| existing_loaders_for_type_id.contains(index))
                {
                    duplicate_extensions.push(extension);
                }

                list.push(loader_index);
            }
            if !duplicate_extensions.is_empty() {
                warn!("Duplicate AssetLoader registered for Asset type `{loader_asset_type_name}` with extensions `{duplicate_extensions:?}`. \
                Loader must be specified in a .meta file in order to load assets of this type with these extensions.");
            }

            self.type_path_to_loader.insert(type_path, loader_index);

            self.type_id_to_loaders
                .entry(loader_asset_type)
                .or_default()
                .push(loader_index);

            self.loaders.push(MaybeAssetLoader::Ready(loader));
        } else {
            let maybe_loader = core::mem::replace(
                self.loaders.get_mut(loader_index).unwrap(),
                MaybeAssetLoader::Ready(loader.clone()),
            );
            match maybe_loader {
                MaybeAssetLoader::Ready(_) => unreachable!(),
                MaybeAssetLoader::Pending { sender, .. } => {
                    IoTaskPool::get()
                        .spawn(async move {
                            let _ = sender.broadcast(loader).await;
                        })
                        .detach();
                }
            }
        }
    }

    /// Pre-register an [`AssetLoader`] that will later be added.
    ///
    /// Assets loaded with matching extensions will be blocked until the
    /// real loader is added.
    pub(crate) fn reserve<L: AssetLoader>(&mut self, extensions: &[&str]) {
        let loader_asset_type = TypeId::of::<L::Asset>();
        let loader_asset_type_name = core::any::type_name::<L::Asset>();
        let type_path = L::type_path();
        // TODO: Allow using the short path of loaders.

        // FORK: as in `push`, a loader type that already has a slot keeps it.
        // `ImagePlugin` preregisters `ImageLoader` in `build` and registers it
        // in `finish`, so a throwaway app built on a shared `AssetServer` comes
        // through here again on every player join; upstream appends a fresh
        // slot each time and shadows the old one, which grows the tables for
        // good.  An existing slot is either still `Pending` (a reservation
        // nobody has filled in yet, which is exactly what this call wants) or
        // already `Ready` (the loader has since been registered, so there is
        // nothing to wait for).  Either way, reserving it again is a no-op.
        if self.type_path_to_loader.contains_key(type_path) {
            return;
        }

        let loader_index = self.loaders.len();

        self.type_path_to_preregistered_loader
            .insert(type_path, loader_index);
        self.type_path_to_loader.insert(type_path, loader_index);

        let existing_loaders_for_type_id = self.type_id_to_loaders.get(&loader_asset_type);
        let mut duplicate_extensions = Vec::new();
        for extension in extensions {
            let list = self
                .extension_to_loaders
                .entry((*extension).into())
                .or_default();

            if !list.is_empty()
                && let Some(existing_loaders_for_type_id) = existing_loaders_for_type_id
                && list
                    .iter()
                    .any(|index| existing_loaders_for_type_id.contains(index))
            {
                duplicate_extensions.push(extension);
            }

            list.push(loader_index);
        }
        if !duplicate_extensions.is_empty() {
            warn!("Duplicate AssetLoader preregistered for Asset type `{loader_asset_type_name}` with extensions `{duplicate_extensions:?}`. \
            Loader must be specified in a .meta file in order to load assets of this type with these extensions.");
        }

        self.type_id_to_loaders
            .entry(loader_asset_type)
            .or_default()
            .push(loader_index);

        let (mut sender, receiver) = async_broadcast::broadcast(1);
        sender.set_overflow(true);
        self.loaders
            .push(MaybeAssetLoader::Pending { sender, receiver });
    }

    /// Get the [`AssetLoader`] by name
    pub(crate) fn get_by_name(&self, name: &str) -> Option<MaybeAssetLoader> {
        let index = self.type_path_to_loader.get(name).copied()?;

        self.get_by_index(index)
    }

    /// Find an [`AssetLoader`] based on provided search criteria
    pub(crate) fn find(
        &self,
        asset_type_id: Option<TypeId>,
        asset_path: &AssetPath<'_>,
    ) -> Option<MaybeAssetLoader> {
        // The presence of a label will affect loader choice
        let label = asset_path.label();

        // Try by asset type
        let candidates = if let Some(type_id) = asset_type_id {
            if label.is_none() {
                Some(self.type_id_to_loaders.get(&type_id)?)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(candidates) = candidates {
            if candidates.is_empty() {
                return None;
            } else if candidates.len() == 1 {
                let index = candidates.first().copied().unwrap();
                return self.get_by_index(index);
            }
        }

        // Asset type is insufficient, use extension information
        let try_extension = |extension| {
            if let Some(indices) = self.extension_to_loaders.get(extension) {
                if let Some(candidates) = candidates {
                    if candidates.is_empty() {
                        indices.last()
                    } else {
                        indices
                            .iter()
                            .rev()
                            .find(|index| candidates.contains(index))
                    }
                } else {
                    indices.last()
                }
            } else {
                None
            }
        };

        // Try extracting the extension from the path
        if let Some(full_extension) = asset_path.get_full_extension() {
            if let Some(&index) = try_extension(full_extension) {
                return self.get_by_index(index);
            }

            // Try secondary extensions from the path
            for extension in AssetPath::iter_secondary_extensions(full_extension) {
                if let Some(&index) = try_extension(extension) {
                    return self.get_by_index(index);
                }
            }
        }

        // Fallback if no resolution step was conclusive
        match candidates?
            .last()
            .copied()
            .and_then(|index| self.get_by_index(index))
        {
            Some(loader) => {
                warn!(
                    "Multiple AssetLoaders found for Asset: {:?}; Path: {:?};",
                    asset_type_id, asset_path
                );
                Some(loader)
            }
            None => {
                warn!(
                    "No AssetLoader found for Asset: {:?}; Path: {:?};",
                    asset_type_id, asset_path
                );
                None
            }
        }
    }

    /// Get the [`AssetLoader`] for a given asset type
    pub(crate) fn get_by_type(&self, type_id: TypeId) -> Option<MaybeAssetLoader> {
        let index = self.type_id_to_loaders.get(&type_id)?.last().copied()?;

        self.get_by_index(index)
    }

    /// Get the [`AssetLoader`] for a given extension
    pub(crate) fn get_by_extension(&self, extension: &str) -> Option<MaybeAssetLoader> {
        let index = self.extension_to_loaders.get(extension)?.last().copied()?;

        self.get_by_index(index)
    }

    /// Get the [`AssetLoader`] for a given path
    pub(crate) fn get_by_path(&self, path: &AssetPath<'_>) -> Option<MaybeAssetLoader> {
        let extension = path.get_full_extension()?;

        let result = core::iter::once(extension)
            .chain(AssetPath::iter_secondary_extensions(extension))
            .filter_map(|extension| self.extension_to_loaders.get(extension)?.last().copied())
            .find_map(|index| self.get_by_index(index))?;

        Some(result)
    }
}

#[derive(Error, Debug, Clone)]
pub(crate) enum GetLoaderError {
    #[error(transparent)]
    CouldNotResolve(#[from] RecvError),
}

#[derive(Clone)]
pub(crate) enum MaybeAssetLoader {
    Ready(Arc<dyn ErasedAssetLoader>),
    Pending {
        sender: async_broadcast::Sender<Arc<dyn ErasedAssetLoader>>,
        receiver: async_broadcast::Receiver<Arc<dyn ErasedAssetLoader>>,
    },
}

impl MaybeAssetLoader {
    pub(crate) async fn get(self) -> Result<Arc<dyn ErasedAssetLoader>, GetLoaderError> {
        match self {
            MaybeAssetLoader::Ready(loader) => Ok(loader),
            MaybeAssetLoader::Pending { mut receiver, .. } => Ok(receiver.recv().await?),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::{format, string::String};
    use core::marker::PhantomData;
    use std::{
        path::Path,
        sync::mpsc::{channel, Receiver, Sender},
    };

    use bevy_reflect::TypePath;
    use bevy_tasks::block_on;

    use crate::Asset;

    use super::*;

    #[derive(Asset, TypePath, Debug)]
    struct A;

    #[derive(Asset, TypePath, Debug)]
    struct B;

    #[derive(Asset, TypePath, Debug)]
    struct C;

    #[derive(TypePath)]
    struct Loader<A: Asset, const N: usize, const E: usize> {
        sender: Sender<()>,
        _phantom: PhantomData<A>,
    }

    impl<T: Asset, const N: usize, const E: usize> Loader<T, N, E> {
        fn new() -> (Self, Receiver<()>) {
            let (tx, rx) = channel();

            let loader = Self {
                sender: tx,
                _phantom: PhantomData,
            };

            (loader, rx)
        }
    }

    impl<T: Asset, const N: usize, const E: usize> AssetLoader for Loader<T, N, E> {
        type Asset = T;

        type Settings = ();

        type Error = String;

        async fn load(
            &self,
            _: &mut dyn crate::io::Reader,
            _: &Self::Settings,
            _: &mut crate::LoadContext<'_>,
        ) -> Result<Self::Asset, Self::Error> {
            self.sender.send(()).unwrap();

            Err(format!(
                "Loaded {}:{}",
                core::any::type_name::<Self::Asset>(),
                N
            ))
        }

        fn extensions(&self) -> &[&str] {
            self.sender.send(()).unwrap();

            match E {
                1 => &["a"],
                2 => &["b"],
                3 => &["c"],
                4 => &["d"],
                _ => &[],
            }
        }
    }

    /// Basic framework for creating, storing, loading, and checking an [`AssetLoader`] inside an [`AssetLoaders`]
    #[test]
    fn basic() {
        let mut loaders = AssetLoaders::default();

        let (loader, rx) = Loader::<A, 1, 0>::new();

        assert!(rx.try_recv().is_err());

        loaders.push(loader);

        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());

        let loader = block_on(
            loaders
                .get_by_name(<Loader<A, 1, 0> as TypePath>::type_path())
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }

    /// Ensure that if multiple loaders have different types but no extensions, they can be found
    #[test]
    fn type_resolution() {
        let mut loaders = AssetLoaders::default();

        let (loader_a1, rx_a1) = Loader::<A, 1, 0>::new();
        let (loader_b1, rx_b1) = Loader::<B, 1, 0>::new();
        let (loader_c1, rx_c1) = Loader::<C, 1, 0>::new();

        loaders.push(loader_a1);
        loaders.push(loader_b1);
        loaders.push(loader_c1);

        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_b1.try_recv().is_ok());
        assert!(rx_c1.try_recv().is_ok());

        let loader = block_on(loaders.get_by_type(TypeId::of::<A>()).unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_b1.try_recv().is_err());
        assert!(rx_c1.try_recv().is_err());

        let loader = block_on(loaders.get_by_type(TypeId::of::<B>()).unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_err());
        assert!(rx_b1.try_recv().is_ok());
        assert!(rx_c1.try_recv().is_err());

        let loader = block_on(loaders.get_by_type(TypeId::of::<C>()).unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_err());
        assert!(rx_b1.try_recv().is_err());
        assert!(rx_c1.try_recv().is_ok());
    }

    /// Ensure that the last loader added is selected
    #[test]
    fn type_resolution_shadow() {
        let mut loaders = AssetLoaders::default();

        let (loader_a1, rx_a1) = Loader::<A, 1, 0>::new();
        let (loader_a2, rx_a2) = Loader::<A, 2, 0>::new();
        let (loader_a3, rx_a3) = Loader::<A, 3, 0>::new();

        loaders.push(loader_a1);
        loaders.push(loader_a2);
        loaders.push(loader_a3);

        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_a2.try_recv().is_ok());
        assert!(rx_a3.try_recv().is_ok());

        let loader = block_on(loaders.get_by_type(TypeId::of::<A>()).unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_err());
        assert!(rx_a2.try_recv().is_err());
        assert!(rx_a3.try_recv().is_ok());
    }

    /// Ensure that if multiple loaders have like types but differing extensions, they can be found
    #[test]
    fn extension_resolution() {
        let mut loaders = AssetLoaders::default();

        let (loader_a1, rx_a1) = Loader::<A, 1, 1>::new();
        let (loader_b1, rx_b1) = Loader::<A, 1, 2>::new();
        let (loader_c1, rx_c1) = Loader::<A, 1, 3>::new();

        loaders.push(loader_a1);
        loaders.push(loader_b1);
        loaders.push(loader_c1);

        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_b1.try_recv().is_ok());
        assert!(rx_c1.try_recv().is_ok());

        let loader = block_on(loaders.get_by_extension("a").unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_b1.try_recv().is_err());
        assert!(rx_c1.try_recv().is_err());

        let loader = block_on(loaders.get_by_extension("b").unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_err());
        assert!(rx_b1.try_recv().is_ok());
        assert!(rx_c1.try_recv().is_err());

        let loader = block_on(loaders.get_by_extension("c").unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_err());
        assert!(rx_b1.try_recv().is_err());
        assert!(rx_c1.try_recv().is_ok());
    }

    /// Ensure that if multiple loaders have like types but differing extensions, they can be found
    #[test]
    fn path_resolution() {
        let mut loaders = AssetLoaders::default();

        let (loader_a1, rx_a1) = Loader::<A, 1, 1>::new();
        let (loader_b1, rx_b1) = Loader::<A, 1, 2>::new();
        let (loader_c1, rx_c1) = Loader::<A, 1, 3>::new();

        loaders.push(loader_a1);
        loaders.push(loader_b1);
        loaders.push(loader_c1);

        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_b1.try_recv().is_ok());
        assert!(rx_c1.try_recv().is_ok());

        let path = AssetPath::from_path(Path::new("asset.a"));

        let loader = block_on(loaders.get_by_path(&path).unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_ok());
        assert!(rx_b1.try_recv().is_err());
        assert!(rx_c1.try_recv().is_err());

        let path = AssetPath::from_path(Path::new("asset.b"));

        let loader = block_on(loaders.get_by_path(&path).unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_err());
        assert!(rx_b1.try_recv().is_ok());
        assert!(rx_c1.try_recv().is_err());

        let path = AssetPath::from_path(Path::new("asset.c"));

        let loader = block_on(loaders.get_by_path(&path).unwrap().get()).unwrap();

        loader.extensions();

        assert!(rx_a1.try_recv().is_err());
        assert!(rx_b1.try_recv().is_err());
        assert!(rx_c1.try_recv().is_ok());
    }

    /// Full resolution algorithm
    #[test]
    fn total_resolution() {
        let mut loaders = AssetLoaders::default();

        let (loader_a1_a, rx_a1_a) = Loader::<A, 1, 1>::new();

        let (loader_b1_b, rx_b1_b) = Loader::<B, 1, 2>::new();

        let (loader_c1_a, rx_c1_a) = Loader::<C, 1, 1>::new();
        let (loader_c1_b, rx_c1_b) = Loader::<C, 1, 2>::new();
        let (loader_c1_c, rx_c1_c) = Loader::<C, 1, 3>::new();

        loaders.push(loader_a1_a);
        loaders.push(loader_b1_b);
        loaders.push(loader_c1_a);
        loaders.push(loader_c1_b);
        loaders.push(loader_c1_c);

        assert!(rx_a1_a.try_recv().is_ok());
        assert!(rx_b1_b.try_recv().is_ok());
        assert!(rx_c1_a.try_recv().is_ok());
        assert!(rx_c1_b.try_recv().is_ok());
        assert!(rx_c1_c.try_recv().is_ok());

        // Type and Extension agree

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<A>()),
                    &AssetPath::from_path(Path::new("asset.a")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_ok());
        assert!(rx_b1_b.try_recv().is_err());
        assert!(rx_c1_a.try_recv().is_err());
        assert!(rx_c1_b.try_recv().is_err());
        assert!(rx_c1_c.try_recv().is_err());

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<B>()),
                    &AssetPath::from_path(Path::new("asset.b")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_err());
        assert!(rx_b1_b.try_recv().is_ok());
        assert!(rx_c1_a.try_recv().is_err());
        assert!(rx_c1_b.try_recv().is_err());
        assert!(rx_c1_c.try_recv().is_err());

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<C>()),
                    &AssetPath::from_path(Path::new("asset.c")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_err());
        assert!(rx_b1_b.try_recv().is_err());
        assert!(rx_c1_a.try_recv().is_err());
        assert!(rx_c1_b.try_recv().is_err());
        assert!(rx_c1_c.try_recv().is_ok());

        // Type should override Extension

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<C>()),
                    &AssetPath::from_path(Path::new("asset.a")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_err());
        assert!(rx_b1_b.try_recv().is_err());
        assert!(rx_c1_a.try_recv().is_ok());
        assert!(rx_c1_b.try_recv().is_err());
        assert!(rx_c1_c.try_recv().is_err());

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<C>()),
                    &AssetPath::from_path(Path::new("asset.b")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_err());
        assert!(rx_b1_b.try_recv().is_err());
        assert!(rx_c1_a.try_recv().is_err());
        assert!(rx_c1_b.try_recv().is_ok());
        assert!(rx_c1_c.try_recv().is_err());

        // Type should override bad / missing extension

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<A>()),
                    &AssetPath::from_path(Path::new("asset.x")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_ok());
        assert!(rx_b1_b.try_recv().is_err());
        assert!(rx_c1_a.try_recv().is_err());
        assert!(rx_c1_b.try_recv().is_err());
        assert!(rx_c1_c.try_recv().is_err());

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<A>()),
                    &AssetPath::from_path(Path::new("asset")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_ok());
        assert!(rx_b1_b.try_recv().is_err());
        assert!(rx_c1_a.try_recv().is_err());
        assert!(rx_c1_b.try_recv().is_err());
        assert!(rx_c1_c.try_recv().is_err());
    }

    /// Ensure that if there is a complete ambiguity in [`AssetLoader`] to use, prefer most recently registered by asset type.
    #[test]
    fn ambiguity_resolution() {
        let mut loaders = AssetLoaders::default();

        let (loader_a1_a, rx_a1_a) = Loader::<A, 1, 1>::new();
        let (loader_a2_a, rx_a2_a) = Loader::<A, 2, 1>::new();
        let (loader_a3_a, rx_a3_a) = Loader::<A, 3, 1>::new();

        loaders.push(loader_a1_a);
        loaders.push(loader_a2_a);
        loaders.push(loader_a3_a);

        assert!(rx_a1_a.try_recv().is_ok());
        assert!(rx_a2_a.try_recv().is_ok());
        assert!(rx_a3_a.try_recv().is_ok());

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<A>()),
                    &AssetPath::from_path(Path::new("asset.a")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_err());
        assert!(rx_a2_a.try_recv().is_err());
        assert!(rx_a3_a.try_recv().is_ok());

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<A>()),
                    &AssetPath::from_path(Path::new("asset.x")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_err());
        assert!(rx_a2_a.try_recv().is_err());
        assert!(rx_a3_a.try_recv().is_ok());

        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<A>()),
                    &AssetPath::from_path(Path::new("asset")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();

        loader.extensions();

        assert!(rx_a1_a.try_recv().is_err());
        assert!(rx_a2_a.try_recv().is_err());
        assert!(rx_a3_a.try_recv().is_ok());
    }

    /// FORK: re-registering a loader type reuses its slot.  Every player who
    /// joins the game builds a throwaway `App` on the one shared
    /// `AssetServer`, so the same loaders are pushed again on every join and
    /// upstream's unconditional append would grow all three tables forever.
    #[test]
    fn re_registering_a_loader_type_reuses_its_slot() {
        let mut loaders = AssetLoaders::default();

        let (first, rx_first) = Loader::<A, 1, 1>::new();
        let (second, rx_second) = Loader::<A, 1, 1>::new();

        loaders.push(first);
        loaders.push(second);

        assert_eq!(loaders.loaders.len(), 1, "the loader list grew");
        assert_eq!(
            loaders.type_id_to_loaders[&TypeId::of::<A>()].len(),
            1,
            "the asset type gained a second candidate, which also costs it the \
             single-loader fast path in `find`"
        );
        assert_eq!(
            loaders.extension_to_loaders["a"].len(),
            1,
            "the extension gained a second candidate"
        );

        // The slot holds the newest instance, matching upstream's
        // last-registration-wins resolution order.
        let loader = block_on(
            loaders
                .get_by_name(<Loader<A, 1, 1> as TypePath>::type_path())
                .unwrap()
                .get(),
        )
        .unwrap();
        loader.extensions();

        assert!(rx_first.try_recv().is_ok(), "`push` asked for extensions once");
        assert!(rx_first.try_recv().is_err());
        assert!(rx_second.try_recv().is_ok(), "the second instance is stored");
        assert!(rx_second.try_recv().is_err());
    }

    /// FORK: a second preregistration of a loader type reuses its slot too.
    /// `ImagePlugin` preregisters in `build` and registers in `finish`, so a
    /// throwaway app built on a shared `AssetServer` runs both again per join.
    #[test]
    fn re_preregistering_a_loader_type_reuses_its_slot() {
        IoTaskPool::get_or_init(Default::default);
        let mut loaders = AssetLoaders::default();

        // Two joins' worth of ImagePlugin: preregister, then register.
        loaders.reserve::<Loader<A, 1, 1>>(&["a"]);
        let (first, _rx_first) = Loader::<A, 1, 1>::new();
        loaders.push(first);
        loaders.reserve::<Loader<A, 1, 1>>(&["a"]);
        let (second, rx_second) = Loader::<A, 1, 1>::new();
        loaders.push(second);

        assert_eq!(loaders.loaders.len(), 1, "the loader list grew");
        assert_eq!(loaders.type_id_to_loaders[&TypeId::of::<A>()].len(), 1);
        assert_eq!(loaders.extension_to_loaders["a"].len(), 1);

        // Re-reserving an already registered loader must not send it back to
        // `Pending`, which would block every load of that type forever.
        let loader = block_on(
            loaders
                .find(
                    Some(TypeId::of::<A>()),
                    &AssetPath::from_path(Path::new("asset.a")),
                )
                .unwrap()
                .get(),
        )
        .unwrap();
        loader.extensions();
        assert!(rx_second.try_recv().is_ok(), "the newest instance is stored");
    }

    /// A `reserve`d slot is still waiting for its loader, so the first `push`
    /// after it must fill that slot in rather than take the re-registration
    /// path -- and a `push` after *that* must still reuse the slot.
    #[test]
    fn re_registering_after_a_preregistration_still_reuses_the_slot() {
        // Filling in a preregistered slot broadcasts the loader from a task.
        IoTaskPool::get_or_init(Default::default);
        let mut loaders = AssetLoaders::default();

        loaders.reserve::<Loader<A, 1, 1>>(&["a"]);
        assert!(matches!(
            loaders.get_by_name(<Loader<A, 1, 1> as TypePath>::type_path()),
            Some(MaybeAssetLoader::Pending { .. })
        ));

        let (first, _rx_first) = Loader::<A, 1, 1>::new();
        loaders.push(first);
        assert!(matches!(
            loaders.get_by_name(<Loader<A, 1, 1> as TypePath>::type_path()),
            Some(MaybeAssetLoader::Ready(_))
        ));

        let (second, rx_second) = Loader::<A, 1, 1>::new();
        loaders.push(second);

        assert_eq!(loaders.loaders.len(), 1, "the loader list grew");
        assert_eq!(loaders.type_id_to_loaders[&TypeId::of::<A>()].len(), 1);
        assert_eq!(loaders.extension_to_loaders["a"].len(), 1);

        let loader = block_on(
            loaders
                .get_by_name(<Loader<A, 1, 1> as TypePath>::type_path())
                .unwrap()
                .get(),
        )
        .unwrap();
        loader.extensions();
        assert!(rx_second.try_recv().is_ok(), "the newest instance is stored");
    }
}
