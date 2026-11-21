use crate::{debug::DebugVariable, id_struct, nonmax::NonMaxU32, world::WorldId};

pub struct Entities {
    world_id: DebugVariable<WorldId>,
    generations: Vec<u32>,
    free_slots: Vec<EntityIndex>,
}

impl Entities {
    pub const RESOURCE_ENTITY: EntityIndex = EntityIndex::new(NonMaxU32::ZERO);
    pub const EXPECTED_PAGE_SIZE: usize = 2usize.pow(Self::EXPECTED_PAGE_SIZE_LOG2 as u32);
    pub const EXPECTED_PAGE_SIZE_LOG2: usize = 10;

    pub fn new(world_id: DebugVariable<WorldId>) -> Self {
        Self {
            world_id,
            generations: Vec::new(),
            free_slots: Vec::new(),
        }
    }

    pub fn reserve(&mut self) -> Entity {
        todo!()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Entity {
    index: EntityIndex,
    generation: u32,
}

const _ENTITY_OPTION_PACKED: () = {
    assert!(std::mem::size_of::<Entity>() == std::mem::size_of::<Option<Entity>>());
};

impl Entity {
    pub const fn new(index: EntityIndex, generation: u32) -> Self {
        Self { index, generation }
    }

    pub const fn index(&self) -> EntityIndex {
        self.index
    }

    pub const fn generation_u32(&self) -> u32 {
        self.generation
    }
}

id_struct!(pub EntityIndex = NonMaxU32);
