use std::{
    borrow::Borrow,
    hash::Hash,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use crate::{
    access::{AccessSet, VariableAccess, checked::AccessCheck},
    bundle::{BundleId, UnsafeBundleComponent},
    component::{ComponentId, Components, LocalComponentId},
    debug::DebugVariable,
    entity::EntityIndex,
    gorc::GorcRef,
    id_struct,
    nonmax::NonMaxU32,
    pure::{ClonePure, DerefPure},
    slice::{Partitioned, SortedSet},
    sparse::SparseSet,
    storage::{
        resource::Resources,
        table::{TableId, Tables, UnsafeTableCell},
    },
    unsafe_cell,
    world::WorldId,
};

id_struct!(pub FragmentId = NonMaxU32);

pub struct Fragments {
    world_id: DebugVariable<WorldId>,
    // NOTE: Once component is added, it's values are immutable.
    component_storages: SparseSet<ComponentId, Option<FragmentComponent>>,
    component_markers: SparseSet<ComponentId, Box<[FragmentComponent]>>,
    fragments: Vec<Box<Fragment>>,
}

#[derive(Debug, Clone, Copy)]
pub struct FragmentComponent {
    pub fid: FragmentId,
    pub local_cid: LocalComponentId,
}

#[derive(Debug, thiserror::Error)]
pub enum FragmentAddingComponentError {
    #[error("Component was already added")]
    AlreadyAdded,
}

impl Fragments {
    pub fn new(world_id: DebugVariable<WorldId>) -> Self {
        Self {
            world_id,
            component_storages: SparseSet::new(),
            component_markers: SparseSet::new(),
            fragments: vec![],
        }
    }

    pub fn push(&mut self, fragment: Box<Fragment>) -> FragmentId {
        let id = FragmentId::new(NonMaxU32::new(self.fragments.len() as _).unwrap());
        self.fragments.push(fragment);
        id
    }

    pub fn add_component(
        &mut self,
        cid: ComponentId,
        storage: Option<FragmentComponent>,
        markers: Box<[FragmentComponent]>,
    ) -> Result<(), FragmentAddingComponentError> {
        if self.component_markers.get(cid).is_some() {
            return Err(FragmentAddingComponentError::AlreadyAdded);
        }
        self.component_storages.insert(cid, storage);
        self.component_markers.insert(cid, markers);
        Ok(())
    }

    pub fn get_component_storage(&self, cid: ComponentId) -> Option<Option<FragmentComponent>> {
        self.component_storages
            .get(cid)
            .map(|v| v.as_ref().cloned())
    }

    pub fn get_component_markers(&self, cid: ComponentId) -> Option<&[FragmentComponent]> {
        self.component_markers.get(cid).map(|v| v.as_ref())
    }

    pub fn get_fragment(&self, fid: FragmentId) -> Option<&Fragment> {
        self.fragments.get(fid.id_usize()).map(Box::as_ref)
    }

    pub fn get_fragment_mut(&mut self, fid: FragmentId) -> Option<&mut Fragment> {
        self.fragments.get_mut(fid.id_usize()).map(Box::as_mut)
    }
}

#[derive(Hash, PartialEq, Eq)]
pub struct FragmentComponents<A = GorcRef<'static, [LocalComponentId]>>(
    pub Partitioned<SortedSet<[LocalComponentId]>, A, 1>,
);

impl<A> Clone for FragmentComponents<A>
where
    A: ClonePure,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl FragmentComponents<Box<[LocalComponentId]>> {
    #[inline(always)] // NOOP
    pub fn as_gorc_ref<'a>(&'a self) -> FragmentComponents<GorcRef<'a, [LocalComponentId]>> {
        // SAFETY: no factual change hence all struct invariants are upheld.
        FragmentComponents(unsafe {
            Partitioned::new_unchecked(*self.0.get_starts(), GorcRef::from_ref(self.0.get_whole()))
        })
    }
}

impl<'a> Borrow<FragmentComponents<&'a [LocalComponentId]>>
    for FragmentComponents<GorcRef<'a, [LocalComponentId]>>
{
    fn borrow(&self) -> &FragmentComponents<&'a [LocalComponentId]> {
        // SAFETY: GorcRef<'a, [T]> and &'a [LocalComponentId] are equivalent.
        unsafe { NonNull::from_ref(self).cast().as_ref() }
    }
}

impl<A> FragmentComponents<A>
where
    A: DerefPure + Deref<Target = [LocalComponentId]>,
{
    pub fn stored(&self) -> &SortedSet<[LocalComponentId]> {
        self.0.get_slice(0)
    }

    pub fn markers(&self) -> &SortedSet<[LocalComponentId]> {
        self.0.get_slice(1)
    }

    pub fn components(&self) -> &[LocalComponentId] {
        self.0.get_whole()
    }
}

impl<'a> From<FragmentComponents<Box<[LocalComponentId]>>>
    for FragmentComponents<GorcRef<'a, [LocalComponentId]>>
{
    fn from(value: FragmentComponents<Box<[LocalComponentId]>>) -> Self {
        let (starts, slice) = value.0.into_parts();
        // SAFETY: no factual change hence all struct invariants are upheld.
        Self(unsafe { Partitioned::new_unchecked(starts, GorcRef::from_box(slice)) })
    }
}

#[repr(transparent)]
pub struct StoredFragmentComponents<A = GorcRef<'static, [LocalComponentId]>>(
    pub FragmentComponents<A>,
);

impl<A> Clone for StoredFragmentComponents<A>
where
    A: ClonePure,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<A> StoredFragmentComponents<A>
where
    A: DerefPure + Deref<Target = [LocalComponentId]>,
{
    pub fn stored(&self) -> &[LocalComponentId] {
        self.0.stored()
    }
}

impl<A> PartialEq for StoredFragmentComponents<A>
where
    A: DerefPure + Deref<Target = [LocalComponentId]>,
{
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.stored(), other.stored()) || self.stored() == other.stored()
    }
}

impl<A> Eq for StoredFragmentComponents<A> where A: DerefPure + Deref<Target = [LocalComponentId]> {}

impl<A> Hash for StoredFragmentComponents<A>
where
    A: DerefPure + Deref<Target = [LocalComponentId]>,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.stored().hash(state);
    }
}

/// Fragment describes any possible storage type.
// TODO: disallow creating the value.
pub enum Fragment {
    Tables(Tables),
    Resources(Resources),
}

pub enum FragmentKind {
    Tables,
    Resources,
}

pub trait FragmentInnerArchetypeIdentifier {
    fn for_tables(self, tables: &mut Tables, components: &Components) -> Option<TableId>;
}

impl FragmentInnerArchetypeIdentifier for EntityIndex {
    fn for_tables(self, tables: &mut Tables, _components: &Components) -> Option<TableId> {
        tables.get_location(self).map(|v| v.table)
    }
}

impl FragmentInnerArchetypeIdentifier for FragmentComponents<Vec<LocalComponentId>> {
    fn for_tables(self, tables: &mut Tables, components: &Components) -> Option<TableId> {
        Some({
            let (entry_builder, mut factory) =
                unsafe { tables.get_table_id_raw_entry_and_factory() };

            let (_, entry) = entry_builder.build(&self, |val| {
                val.0.get_starts() == self.0.get_starts()
                    && val.0.get_whole().deref() == self.0.get_whole().as_slice()
            });

            let (starts, vec) = self.0.into_parts();

            *entry
                .or_insert_with(|| {
                    let owned = unsafe {
                        FragmentComponents(Partitioned::new_unchecked(
                            starts,
                            GorcRef::from_box(vec.into_boxed_slice()),
                        ))
                    };
                    let id = factory(&owned, components);
                    (owned, id)
                })
                .1
        })
    }
}

impl Fragment {
    pub fn all_components(&self) -> impl Iterator<Item = ComponentId> {
        match self {
            Fragment::Tables(tables) => tables.all_components(),
            Fragment::Resources(resources) => resources.all_components(),
        }
    }

    pub fn stored_fragment_components_for(
        &self,
        index: EntityIndex,
    ) -> Option<&StoredFragmentComponents> {
        match self {
            Fragment::Tables(tables) => {
                let location = tables.get_location(index)?;
                Some(
                    tables
                        .get_table(location.table)?
                        .stored_fragment_components(),
                )
            }
            Fragment::Resources(resources) => {
                // each entity holds the same components
                let mut ids = resources
                    .all_components()
                    .map(|v| LocalComponentId::new(v.id()))
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                todo!()
            }
        }
    }

    /// # Safety
    /// [`BundleId`] must correspond to a bundle which actually results in this transition.
    pub unsafe fn store_bundle_edge<I1, I2>(
        &mut self,
        from: I1,
        to: I2,
        bundle_id: BundleId,
        components: &Components,
    ) where
        I1: FragmentInnerArchetypeIdentifier,
        I2: FragmentInnerArchetypeIdentifier,
    {
        match self {
            Fragment::Tables(tables) => {
                let from = from.for_tables(tables, components);
                let to = to.for_tables(tables, components);
                // SAFETY: The caller's contract.
                unsafe { tables.store_bundle_edge(bundle_id, from, to) };
            }
            Fragment::Resources(resources) => todo!(),
        }
    }

    pub unsafe fn relocate_entity(
        &mut self,
        bundle_id: BundleId,
        index: EntityIndex,
        // TODO: SortedSet
        bundle: impl Iterator<Item = UnsafeBundleComponent>,
    ) {
        match self {
            Fragment::Tables(tables) => {
                let from_id = tables.get_location(index).map(|v| v.table);
                let to_id = unsafe {
                    tables
                        .get_bundle_edge(bundle_id, from_id)
                        .unwrap_unchecked()
                };
                unsafe { tables.relocate_entity(from_id, to_id, bundle, index) };
            }
        }
    }

    /// # Safety
    /// - `self` must be [`Self::Tables`].
    pub fn get_table_id<'a, A>(
        &mut self,
        fragment_components: &FragmentComponents<A>,
    ) -> Option<TableId>
    where
        FragmentComponents<GorcRef<'a, [LocalComponentId]>>: Borrow<FragmentComponents<A>>,
        A: Hash + PartialEq + Eq + PartialOrd + Ord + 'a,
    {
        match self {
            Fragment::Tables(tables) => tables.get_table_id(fragment_components),
            _ => {
                // SAFETY: The caller's contract.
                unsafe { std::hint::unreachable_unchecked() };
            }
        }
    }

    /// # Safety
    /// - `self` must be [`Self::Tables`].
    /// - All local components ids in `fragment_components` must exist within this fragment and
    /// exist within this `components` (via corresponding mapping of this fragment).
    pub unsafe fn insert_or_get_table_id(
        &mut self,
        fragment_components: FragmentComponents,
        components: &Components,
    ) -> TableId {
        match self {
            // SAFETY: The caller's contract.
            Fragment::Tables(tables) => unsafe {
                tables.insert_or_get_table_id(fragment_components, components)
            },
            _ => {
                // SAFETY: The caller's contract.
                unsafe { std::hint::unreachable_unchecked() };
            }
        }
    }
}

unsafe_cell!(pub UnsafeFragmentsCell<'a> = Fragments, &'a AccessSet<'a>);

impl<'a, AC> UnsafeFragmentsCell<'a, AC>
where
    AC: AccessCheck<Inner = &'a AccessSet<'a>>,
{
    /// # Safety
    /// You must have **any** access to [`Fragments`].
    pub unsafe fn get_fragment_cell(
        &self,
        fid: FragmentId,
    ) -> AC::Res<Option<UnsafeFragmentCell<'a, AC::Map<(&'a AccessSet<'a>, FragmentId)>>>> {
        self.access().test_and_get(
            |v| !v.fragments.is_none(),
            || {
                // SAFETY: pointer is created from a valid reference.
                Some(unsafe {
                    UnsafeFragmentCell::new(
                        NonNull::from_ref(self.ptr().as_ref().get_fragment(fid)?),
                        self.access().ref_map(|access| (*access, fid)),
                    )
                })
            },
        )
    }

    pub unsafe fn get_metadata_ref(&self) -> AC::Res<UnsafeFragmentComponentMetadataRef<'a>> {
        self.access().test_and_get(
            |s| s.components_metadata.contains(VariableAccess::NonExclusive),
            || unsafe { UnsafeFragmentComponentMetadataRef::new(self.ptr()) },
        )
    }
}

unsafe_cell!(pub UnsafeFragmentCell<'a> = Fragment, (&'a AccessSet<'a>, FragmentId));

impl<'a, AC> UnsafeFragmentCell<'a, AC>
where
    AC: AccessCheck<Inner = (&'a AccessSet<'a>, FragmentId)>,
{
    /// # Safety
    /// - `self` must have permission to access immutable reference of its' fragment.
    /// - no other conflicting references must exist to the fragment.
    pub unsafe fn as_ref(&self) -> AC::Res<&'a Fragment> {
        self.access().test_and_get(
            |v| v.0.fragments.can_access_fragment_immutably(v.1),
            || unsafe { self.ptr().as_ref() },
        )
    }

    /// # Safety
    /// `self` must have permission to create mutable reference of its' fragment.
    pub unsafe fn as_mut(&self) -> AC::Res<&'a mut Fragment> {
        self.access().test_and_get(
            |v| v.0.fragments.can_access_fragment_mutably(v.1),
            || unsafe { self.ptr().as_mut() },
        )
    }

    pub fn kind(&self) -> FragmentKind {
        // SAFETY: The pointer is always valid and no mutable alias exists.
        match unsafe { self.ptr().as_ref() } {
            Fragment::Tables(_) => FragmentKind::Tables,
        }
    }

    pub unsafe fn get_table(
        &self,
        tid: TableId,
    ) -> AC::Res<UnsafeTableCell<'a, AC::Map<(&'a AccessSet<'a>, FragmentId, TableId)>>> {
        self.access().test_and_get(
            |v| v.0.fragments.any_fragment_access(v.1),
            || unsafe {
                UnsafeTableCell::new(
                    match self.ptr().as_mut() {
                        Fragment::Tables(tables) => tables.get_table_ptr_unchecked(tid),
                        _ => std::hint::unreachable_unchecked(),
                    },
                    self.access().ref_map(|v| (v.0, v.1, tid)),
                )
            },
        )
    }
}

pub struct UnsafeFragmentComponentMetadataRef<'a> {
    ptr: NonNull<Fragments>,
    _marker: PhantomData<&'a Fragments>,
}

impl<'a> UnsafeFragmentComponentMetadataRef<'a> {
    pub unsafe fn new(ptr: NonNull<Fragments>) -> Self {
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// # Safety
    /// Only component metadata variables may be accessed.
    unsafe fn get_ref(&self) -> &Fragments {
        unsafe { self.ptr.as_ref() }
    }

    pub fn get_component_storage(&self, cid: ComponentId) -> Option<Option<FragmentComponent>> {
        unsafe { self.get_ref() }.get_component_storage(cid)
    }

    pub fn get_component_markers(&self, cid: ComponentId) -> Option<&[FragmentComponent]> {
        unsafe { self.get_ref() }.get_component_markers(cid)
    }
}
