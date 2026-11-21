use std::{alloc::Layout, any::TypeId};

use hashbrown::HashMap;

use crate::{debug::DebugVariable, gorc::GorcRef, id_struct, nonmax::NonMaxU32, world::WorldId};

id_struct!(pub ComponentId = NonMaxU32);
id_struct!(pub LocalComponentId = NonMaxU32);

pub struct ComponentData {
    pub(crate) layout: Layout,
    /// # Safety
    /// - The given argument must correspond to the underlying type of this component data.
    /// - It is must be droppable via the given `drop` function. When the type is a rust one, by
    /// default it would be [`std::ptr::drop_in_place`]
    /// - inherits [`std::ptr::drop_in_place`] requirements for the underlying type.
    pub(crate) drop: Option<unsafe extern "C" fn(*mut u8)>,
    pub(crate) name: GorcRef<'static, str>,
}

impl std::fmt::Debug for ComponentData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ComponentData")
            .field(&self.name.as_ref())
            .finish()
    }
}

unsafe extern "C" fn drop_fn<T>(ptr: *mut u8) {
    // SAFETY: The caller's contract.
    unsafe { std::ptr::drop_in_place::<T>(ptr.cast()) };
}

impl ComponentData {
    pub fn new<T>() -> Self
    where
        T: Sync + Send,
    {
        Self {
            layout: Layout::new::<T>(),
            drop: std::mem::needs_drop::<T>()
                .then(|| drop_fn::<T> as unsafe extern "C" fn(*mut u8)),
            name: GorcRef::from_ref(std::any::type_name::<T>()),
        }
    }
}

#[derive(Debug)]
pub struct Components {
    world_id: DebugVariable<WorldId>,
    data: Vec<GorcRef<'static, ComponentData>>,
}

impl Components {
    pub fn new(world_id: DebugVariable<WorldId>) -> Self {
        Self {
            world_id,
            data: vec![],
        }
    }

    pub fn get(&self, id: ComponentId) -> Option<&GorcRef<'static, ComponentData>> {
        self.data.get(id.id_usize())
    }

    pub fn push(&mut self, data: GorcRef<'static, ComponentData>) -> Option<ComponentId> {
        let id = ComponentId::new(NonMaxU32::new(self.data.len() as u32)?);
        self.data.push(data);
        Some(id)
    }
}

pub struct RustComponents {
    map: HashMap<TypeId, ComponentId>,
}

impl RustComponents {
    // Unfortunately, it is necessary to have a reserved component id for RustComponents. Imagine
    // not having it, then it the struct would have to be somehow baked into the world, which is not
    // necessary for projects using schmecs within other (than rust) languages.
    //
    // So to use rust schmecs, this struct will have to be inserted into Resources for
    // RESOURCE_ENTITY.
    pub const RESERVED_CID: ComponentId = ComponentId::new(NonMaxU32::from_inverse(1).unwrap());

    pub fn get_cid(&self, tid: TypeId) -> Option<ComponentId> {
        self.map.get(&tid).cloned()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        component::{ComponentData, Components},
        gorc::GorcRef,
        world::ZERO_WORLD_ID,
    };

    #[test]
    fn components_test() {
        let mut components = Components::new(ZERO_WORLD_ID);
        let id = components
            .push(GorcRef::from_box(Box::new(ComponentData::new::<u32>())))
            .unwrap();
        assert!(components.get(id).is_some());
    }
}
