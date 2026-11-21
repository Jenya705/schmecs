//! Bundle is a set of components added/removed to/of a single entity. In other ecs libraries bundles might
//! be less restrictive than here.

use std::{
    cell::UnsafeCell,
    collections::{BTreeMap, HashMap},
    ptr::NonNull,
};

use dashmap::DashMap;

use crate::{
    component::{ComponentId, Components, LocalComponentId},
    debug::DebugVariable,
    entity::EntityIndex,
    fragment::{Fragment, FragmentComponents, FragmentId, UnsafeFragmentsCell},
    gorc::GorcRef,
    id_struct,
    iter::{IteratorExt, SortedCombinedIter},
    nonmax::NonMaxU32,
    slice::Partitioned,
    sparse::SparseSet,
    vec::PushOnlyUninitAtomicVec,
    world::WorldId,
};

id_struct!(pub BundleId = u32);

pub struct Bundles {
    world_id: DebugVariable<WorldId>,
    bundles: DashMap<BundleTopology<'static>, BundleId>,
    tapes: PushOnlyUninitAtomicVec<UnsafeBundleTape<'static>>,
}

impl Bundles {
    pub fn new(world_id: DebugVariable<WorldId>) -> Self {
        Self {
            world_id,
            bundles: DashMap::new(),
            tapes: PushOnlyUninitAtomicVec::new(),
        }
    }

    fn compute_unsafe_bundle_tape(
        topology: &BundleTopology,
        fragments: &UnsafeFragmentsCell,
    ) -> UnsafeBundleTape<'static> {
        let mut fragment_local_components: SparseSet<FragmentId, Vec<UnsafeBundleTapeComponent>> =
            SparseSet::new();

        let mut dense_fragment_ids: Vec<(
            FragmentId,
            GorcRef<'static, [UnsafeBundleTapeComponent]>,
        )> = vec![];

        let metadata = unsafe { fragments.get_metadata_ref() };

        for (i, added) in topology.adding().iter().cloned().enumerate() {
            if let Some(Some(fc)) = metadata.get_component_storage(added) {
                let component = UnsafeBundleTapeComponent::adding(fc.local_cid, unsafe {
                    NonMaxU32::new_unchecked(i as _)
                });

                fragment_local_components
                    .get_or_insert_mut(fc.fid, || {
                        dense_fragment_ids.push((fc.fid, GorcRef::from_ref(&[])));
                        vec![]
                    })
                    // SAFETY: There are maximum u32::MAX - 1 components.
                    .push(component);
            }
        }

        for _removed in topology.removing().iter().cloned() {
            let _ = ();
        }

        dense_fragment_ids.sort_unstable_by_key(|(fid, _)| *fid);

        for (fid, components) in dense_fragment_ids.iter_mut() {
            // SAFETY: All fragment ids from dense_fragment_ids exist in fragment_local_components.
            let val = GorcRef::from_box(
                unsafe { fragment_local_components.take(*fid).unwrap_unchecked() }
                    .into_boxed_slice(),
            );

            // We know that it is a placeholder we are replacing which is built using `from_ref`
            // thus the drop function does nothing and thus it doesn't have to be executed (no
            // memory leak happens anyway).
            std::mem::forget(std::mem::replace(components, val));
        }

        let tape = UnsafeBundleTape::new(GorcRef::from_box(dense_fragment_ids.into_boxed_slice()));

        tape
    }

    /// # Safety
    /// All fragments used in this bundle must be safe to access mutably via `fragments`.
    unsafe fn compute_and_save_bundle_transitions(
        topology: &BundleTopology,
        fragments: &UnsafeFragmentsCell,
        index: EntityIndex,
        bundle_id: BundleId,
        components: &Components,
    ) {
        let mut fragment_dense_ids: BTreeMap<FragmentId, u32> = BTreeMap::new();

        struct FragmentData<'a> {
            cell: UnsafeCell<&'a mut Fragment>,
            cur_stored: std::slice::Iter<'a, LocalComponentId>,
            cur_markers: std::slice::Iter<'a, LocalComponentId>,
            res_stored: usize,
            res: Vec<LocalComponentId>,
        }

        let mut fragment_data: Vec<FragmentData> = Vec::new();

        macro_rules! get_fragment_data {
            ($id: expr) => {{
                let id: FragmentId = $id;
                let dense_id = *fragment_dense_ids.entry(id).or_insert_with(|| {
                    // SAFETY:
                    // - `id` is within this `Fragments` as it was got from it.
                    // - The caller ensures that we have some access to fragments.
                    let cell = unsafe { fragments.get_fragment_cell(id).unwrap_unchecked() };
                    // SAFETY: The caller ensures that we have mutable access to the fragment.
                    let fragment = UnsafeCell::new(unsafe { cell.as_mut() });
                    let fragment_ref = unsafe { &mut *fragment.get() };
                    let stored = fragment_ref
                        .stored_fragment_components_for(index)
                        .map(|v| &v.0);
                    let data = FragmentData {
                        cur_stored: stored.map(|v| v.stored().iter()).unwrap_or([].iter()),
                        cur_markers: stored.map(|v| v.markers().iter()).unwrap_or([].iter()),
                        cell: fragment,
                        res_stored: 0,
                        res: vec![],
                    };
                    let dense_id = fragment_data.len() as u32;
                    fragment_data.push(data);
                    dense_id
                });
                // SAFETY: all ids from fragment_dense_ids were created from indicies in
                // fragment_data.
                unsafe { fragment_data.get_mut(dense_id as usize).unwrap_unchecked() }
            }};
        }

        macro_rules! push_lcids {
            ($data: expr, $fd_iter: ident, $originating: expr, $fc_lcid: expr) => {
                let fd_iter = &mut $data.$fd_iter;
                while matches!(fd_iter.clone().next(), Some(&v) if v < $fc_lcid) {
                    // SAFETY: Was Some right before that, the iterator didn't change.
                    let &v = unsafe { fd_iter.next().unwrap_unchecked() };
                    $data.res.push(v);
                }

                let _ = fd_iter.next_if(|&&v| v == $fc_lcid);

                if !$originating.get(1) {
                    $data.res.push($fc_lcid);
                }
            }
        }

        let mut iters = [
            topology.adding().iter().cloned(),
            topology.removing().iter().cloned(),
        ];

        let mut iters_c = iters.clone();

        let mut compound_iter = SortedCombinedIter::new(&mut iters_c);

        loop {
            let Some((cid, originating)) = compound_iter.next() else {
                break;
            };

            if let Some(Some(fc)) =
                unsafe { fragments.get_metadata_ref() }.get_component_storage(cid)
            {
                let data = get_fragment_data!(fc.fid);

                push_lcids!(data, cur_stored, originating, fc.local_cid);

                data.res_stored = data.res.len();
            }
        }

        let mut compound_iter = SortedCombinedIter::new(&mut iters);

        loop {
            let Some((cid, originating)) = compound_iter.next() else {
                break;
            };

            if let Some(markers) =
                unsafe { fragments.get_metadata_ref() }.get_component_markers(cid)
            {
                for fc in markers {
                    let data = get_fragment_data!(fc.fid);

                    if data.cur_stored.clone().next().is_some() {
                        data.res.extend(&mut data.cur_stored);
                        data.res_stored = data.res.len();
                    }

                    push_lcids!(data, cur_markers, originating, fc.local_cid);
                }
            }
        }

        for data in fragment_data {
            let FragmentData {
                cell,
                cur_stored,
                cur_markers,
                mut res_stored,
                mut res,
            } = data;

            if cur_stored.clone().next().is_some() {
                res.extend(cur_stored);
                res_stored = res.len();
            }
            if cur_markers.clone().next().is_some() {
                res.extend(cur_markers);
            }

            // SAFETY: The caller ensures that we have mutable access to the fragment.
            unsafe {
                cell.into_inner().store_bundle_edge(
                    index,
                    // SAFETY: data.res_stored is the split point where vector splits into two
                    // sorted slices as per the algorithm above.
                    FragmentComponents(Partitioned::new_unchecked([res_stored], res)),
                    bundle_id,
                    components,
                );
            }
        }
    }

    pub fn find_or_compute_bundle_id(
        &self,
        topology: BundleTopology<'static>,
        fragments: &UnsafeFragmentsCell,
    ) -> (BundleId, bool) {
        match self.bundles.entry(topology) {
            dashmap::Entry::Occupied(occupied_entry) => (*occupied_entry.get(), false),
            dashmap::Entry::Vacant(vacant_entry) => {
                let id = BundleId::new(self.tapes.push(Self::compute_unsafe_bundle_tape(
                    vacant_entry.key(),
                    fragments,
                )) as _);

                vacant_entry.insert(id);

                (id, true)
            }
        }
    }

    pub unsafe fn relocate_entity(
        &self,
        topology: BundleTopology<'static>,
        added_components: &[NonNull<u8>],
        index: EntityIndex,
        components: &Components,
        fragments: &UnsafeFragmentsCell,
    ) {
        let (bundle_id, new) = self.find_or_compute_bundle_id(topology.clone(), fragments);

        if new {
            unsafe {
                Self::compute_and_save_bundle_transitions(
                    &topology, fragments, index, bundle_id, components,
                );
            }
        }

        // SAFETY: Was added right before.
        let tape = unsafe { self.tapes.get_unchecked(bundle_id.id_usize()) };

        let mut iter = unsafe { UnsafeBundleIter::new(tape, added_components) };
        while let Some((fid, bundle)) = iter.next() {
            unsafe {
                fragments
                    .get_fragment_cell(fid)
                    .unwrap_unchecked()
                    .as_mut()
                    .relocate_entity(bundle_id, index, bundle);
            }
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone)]
pub struct BundleTopology<'a> {
    adding: usize,
    ids: GorcRef<'a, [ComponentId]>,
}

impl<'a> BundleTopology<'a> {
    pub unsafe fn new(adding: usize, ids: GorcRef<'a, [ComponentId]>) -> Self {
        Self { adding, ids }
    }

    pub fn adding(&self) -> &[ComponentId] {
        &self.ids[0..self.adding]
    }

    pub fn removing(&self) -> &[ComponentId] {
        &self.ids[self.adding..]
    }
}

pub struct UnsafeBundleTape<'a> {
    fragments: GorcRef<'a, [(FragmentId, GorcRef<'a, [UnsafeBundleTapeComponent]>)]>,
}

impl<'a> UnsafeBundleTape<'a> {
    pub(crate) fn new(
        fragments: GorcRef<'a, [(FragmentId, GorcRef<'a, [UnsafeBundleTapeComponent]>)]>,
    ) -> Self {
        Self { fragments }
    }
}

#[derive(Clone, Copy)]
pub struct UnsafeBundleTapeComponent {
    component_id: LocalComponentId,
    ptr_id: Option<NonMaxU32>,
}

impl UnsafeBundleTapeComponent {
    pub const fn adding(component_id: LocalComponentId, ptr_id: NonMaxU32) -> Self {
        Self {
            component_id: component_id,
            ptr_id: Some(ptr_id),
        }
    }

    pub const fn removing(component_id: LocalComponentId) -> Self {
        Self {
            component_id: component_id,
            ptr_id: None,
        }
    }

    pub const fn as_bundle_component(&self, ptrs: &[NonNull<u8>]) -> UnsafeBundleComponent {
        match (self.component_id, self.ptr_id) {
            (cid, None) => UnsafeBundleComponent::Remove { component_id: cid },
            (cid, Some(ptr)) => UnsafeBundleComponent::Add {
                component_id: cid,
                ptr: ptrs[ptr.get() as usize],
            },
        }
    }
}

const _: () = {
    assert!(std::mem::size_of::<UnsafeBundleTapeComponent>() == 8);
};

pub struct UnsafeBundleIter<'a> {
    iter: std::slice::Iter<'a, (FragmentId, GorcRef<'a, [UnsafeBundleTapeComponent]>)>,
    ptrs: &'a [NonNull<u8>],
}

impl<'a> UnsafeBundleIter<'a> {
    pub unsafe fn new(tape: &'a UnsafeBundleTape, ptrs: &'a [NonNull<u8>]) -> Self {
        Self {
            iter: tape.fragments.iter(),
            ptrs,
        }
    }

    pub fn next(&mut self) -> Option<(FragmentId, UnsafeBundleFragmentIter<'_>)> {
        self.iter.next().map(|(fid, v)| {
            (
                *fid,
                UnsafeBundleFragmentIter {
                    iter: v.iter(),
                    ptrs: self.ptrs,
                },
            )
        })
    }
}

pub struct UnsafeBundleFragmentIter<'a> {
    iter: std::slice::Iter<'a, UnsafeBundleTapeComponent>,
    ptrs: &'a [NonNull<u8>],
}

impl<'a> Iterator for UnsafeBundleFragmentIter<'a> {
    type Item = UnsafeBundleComponent;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|v| v.as_bundle_component(self.ptrs))
    }
}

pub enum UnsafeBundleComponent {
    Add {
        component_id: LocalComponentId,
        ptr: NonNull<u8>,
    },
    Remove {
        component_id: LocalComponentId,
    },
}

pub struct BundleEdges<I> {
    transitions: HashMap<(BundleId, I), I>,
}

impl<I> BundleEdges<I>
where
    I: std::hash::Hash + PartialEq + Eq,
{
    pub fn new() -> Self {
        Self {
            transitions: HashMap::new(),
        }
    }

    pub fn insert(&mut self, bundle: BundleId, from: I, to: I) {
        self.transitions.insert((bundle, from), to);
    }

    pub fn get(&self, bundle: BundleId, from: I) -> Option<&I> {
        self.transitions.get(&(bundle, from))
    }
}
