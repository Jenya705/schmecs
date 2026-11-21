use std::{ptr::NonNull, sync::atomic::AtomicU32};

use crate::{
    access::{AccessSet, VariableAccess, checked::AccessCheck},
    bundle::Bundles,
    component::Components,
    debug::DebugVariable,
    entity::Entities,
    fragment::{Fragments, UnsafeFragmentsCell},
    unsafe_cell,
};

pub struct WorldIdGlobalCounter(AtomicU32);

pub type WorldId = u32;

#[cfg(test)]
pub const ZERO_WORLD_ID: DebugVariable<WorldId> = DebugVariable::new(0);

impl WorldIdGlobalCounter {
    pub fn reserve(&self) -> WorldId {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

static DEBUG_WORLD_ID_GLOBAL_COUNTER: DebugVariable<WorldIdGlobalCounter> =
    DebugVariable::new(WorldIdGlobalCounter(AtomicU32::new(0)));

impl DebugVariable<WorldIdGlobalCounter> {
    pub fn reserve(&self) -> DebugVariable<WorldId> {
        self.as_ref().map(|v| v.reserve())
    }
}

pub struct World {
    pub(crate) id: DebugVariable<WorldId>,
    pub(crate) bundles: Bundles,
    pub(crate) components: Components,
    pub(crate) entities: Entities,
    pub(crate) fragments: Fragments,
}

impl World {
    pub fn world_id(&self) -> DebugVariable<WorldId> {
        self.id
    }

    pub fn new() -> Self {
        let id = DEBUG_WORLD_ID_GLOBAL_COUNTER.reserve();
        Self {
            id,
            bundles: Bundles::new(id),
            components: Components::new(id),
            entities: Entities::new(id),
            fragments: Fragments::new(id),
        }
    }
}

unsafe_cell!(pub UnsafeWorldCell<'a> = World, &'a AccessSet<'a>);

impl<'a, AC> UnsafeWorldCell<'a, AC>
where
    AC: AccessCheck<Inner = &'a AccessSet<'a>>,
{
    unsafe fn get_field_ptr<T>(&self, off: usize) -> NonNull<T> {
        unsafe { self.ptr().cast().byte_add(off) }
    }

    fn bundles_ptr(&self) -> NonNull<Bundles> {
        // SAFETY: World is safe to dereference, thus we can access `bundles` (or in this case move
        // the pointer).
        unsafe { self.get_field_ptr(std::mem::offset_of!(World, bundles)) }
    }

    fn components_ptr(&self) -> NonNull<Components> {
        unsafe { self.get_field_ptr(std::mem::offset_of!(World, components)) }
    }

    fn entities_ptr(&self) -> NonNull<Entities> {
        // SAFETY: World is safe to dereference, thus we can access `bundles` (or in this case move
        // the pointer).
        unsafe { self.get_field_ptr(std::mem::offset_of!(World, entities)) }
    }

    fn fragments_ptr(&self) -> NonNull<Fragments> {
        unsafe { self.get_field_ptr(std::mem::offset_of!(World, fragments)) }
    }

    /// # Safety
    /// This [`UnsafeWorldCell`] must have permission to access [`Bundles`] exclusively or
    /// inexclusively.
    pub unsafe fn bundles(&self) -> AC::Res<&'a Bundles> {
        self.access().test_and_get(
            |v| v.bundles.contains(VariableAccess::NonExclusive),
            || unsafe { self.bundles_ptr().as_ref() },
        )
    }

    /// # Safety
    /// This [`UnsafeWorldCell`] must have permission to access [`Bundles`] exclusively.
    pub unsafe fn bundles_mut(&self) -> AC::Res<&'a mut Bundles> {
        self.access().test_and_get(
            |v| v.bundles.contains(VariableAccess::Exclusive),
            || unsafe { self.bundles_ptr().as_mut() },
        )
    }

    pub unsafe fn entities(&self) -> AC::Res<&'a Entities> {
        self.access().test_and_get(
            |v| v.entities.contains(VariableAccess::NonExclusive),
            || unsafe { self.entities_ptr().as_ref() },
        )
    }

    pub unsafe fn entities_mut(&self) -> AC::Res<&'a mut Entities> {
        self.access().test_and_get(
            |v| v.entities.contains(VariableAccess::Exclusive),
            || unsafe { self.entities_ptr().as_mut() },
        )
    }

    pub unsafe fn components(&self) -> AC::Res<&'a Components> {
        self.access().test_and_get(
            |v| v.components_metadata.contains(VariableAccess::NonExclusive),
            || unsafe { self.components_ptr().as_ref() },
        )
    }

    pub unsafe fn components_mut(&self) -> AC::Res<&'a mut Components> {
        self.access().test_and_get(
            |v| v.components_metadata.contains(VariableAccess::Exclusive),
            || unsafe { self.components_ptr().as_mut() },
        )
    }

    pub fn fragments_cell(&self) -> UnsafeFragmentsCell<'a, AC::Map<&'a AccessSet<'a>>> {
        unsafe { UnsafeFragmentsCell::new(self.fragments_ptr(), self.access().clone_map()) }
    }
}
