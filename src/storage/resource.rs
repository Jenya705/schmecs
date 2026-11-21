//! Resources still store components but in a really non data oriented way, it tries to minimize the
//! memory usage, expecting caller to cache results of queries (like caching [`ResourceId`]).
//!
//! This way 'resources' (i.e., singletone components) won't have to interfere with other fragments,
//! but rather can have their own singletone optimized fragment.
use std::{collections::BTreeMap, ptr::NonNull};

use crate::{
    component::{ComponentData, ComponentId},
    entity::EntityIndex,
    gorc::GorcRef,
    id_struct,
};

pub struct Resources {
    // TODO: Atomic
    resources: Vec<Option<Resource>>,
    free: Vec<ResourceId>,
    rids: BTreeMap<(EntityIndex, ComponentId), ResourceId>,
}

struct Resource {
    ptr: NonNull<u8>,
    cid: ComponentId,
    entity: EntityIndex,
    data: GorcRef<'static, ComponentData>,
}

id_struct!(pub ResourceId = u32);

impl Resources {}
