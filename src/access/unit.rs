use crate::{
    access::AccessConflict,
    component::ComponentId,
    fragment::{FragmentId, UnsafeFragmentComponentMetadataRef},
    gorc::GorcRef,
    slice::SortedSet,
};

pub enum AccessUnit {
    Bundles(ReferenceKind),
    Components(ReferenceKind),
    Entities(ReferenceKind),
    Fragments(ReferenceKind),
    Fragment(FragmentId, ReferenceKind),
    Component(ComponentId, ComponentReferenceKind, EntityFilter),
}

impl AccessUnit {
    /// Whether these two access units conflict when used **simultaneously**. No units conflict when
    /// used sequentially.
    pub fn conflicts(
        &self,
        other: &Self,
        fragment_metadata: UnsafeFragmentComponentMetadataRef,
    ) -> AccessConflict {
        match self {
            AccessUnit::Bundles(reference_kind) => AccessConflict::yes_if_true(
                matches!(other, Self::Bundles(b) if b.conflicts(*reference_kind).yes()),
            ),
            AccessUnit::Components(reference_kind) => AccessConflict::yes_if_true(
                matches!(other, Self::Components(b) if b.conflicts(*reference_kind).yes()),
            ),
            AccessUnit::Entities(reference_kind) => {
                AccessConflict::yes_if_true(match reference_kind {
                    ReferenceKind::Mutable => {
                        !matches!(other, AccessUnit::Bundles(_) | AccessUnit::Components(_))
                    }
                    ReferenceKind::Immutable => {
                        matches!(other, AccessUnit::Entities(ReferenceKind::Mutable))
                    }
                })
            }
            AccessUnit::Fragments(reference_kind) => {
                AccessConflict::yes_if_true(match reference_kind {
                    ReferenceKind::Mutable => {
                        !matches!(other, AccessUnit::Bundles(_) | AccessUnit::Components(_))
                    }
                    ReferenceKind::Immutable => match other {
                        AccessUnit::Bundles(_) => false,
                        AccessUnit::Components(_) => false,
                        AccessUnit::Entities(reference_kind) => {
                            matches!(reference_kind, ReferenceKind::Mutable)
                        }
                        AccessUnit::Fragments(reference_kind) => {
                            matches!(reference_kind, ReferenceKind::Mutable)
                        }
                        AccessUnit::Fragment(fragment_id, reference_kind) => {
                            matches!(reference_kind, ReferenceKind::Mutable)
                        }
                        AccessUnit::Component(
                            component_id,
                            component_reference_kind,
                            entity_filter,
                        ) => matches!(
                            component_reference_kind,
                            ComponentReferenceKind::Mutable
                                | ComponentReferenceKind::DiscriminantMutable
                        ),
                    },
                })
            }
            // despite an immutable access, you cannot actually insert/remove components if they are
            // markers of other fragments.
            AccessUnit::Fragment(fragment_id, reference_kind) => match other {
                AccessUnit::Fragment(fragment_id2, reference_kind2) => {
                    if fragment_id == fragment_id2 {
                        reference_kind.conflicts(*reference_kind2)
                    } else {
                        AccessConflict::No
                    }
                }
                AccessUnit::Component(component_id2, component_reference_kind2, entity_filter) => {
                    match component_reference_kind2 {
                        ComponentReferenceKind::DiscriminantMutable => {
                            // no loop
                            'ret: loop {
                                if let Some(Some(fc)) =
                                    fragment_metadata.get_component_storage(*component_id2)
                                {
                                    if fc.fid == *fragment_id {
                                        break AccessConflict::Yes;
                                    }
                                }

                                if let Some(iter) =
                                    fragment_metadata.get_component_markers(*component_id2)
                                {
                                    for v in iter {
                                        if v.fid == *fragment_id {
                                            break 'ret AccessConflict::Yes;
                                        }
                                    }
                                }

                                break AccessConflict::No;
                            }
                        }
                        ComponentReferenceKind::Mutable => {
                            if let Some(Some(fc)) =
                                fragment_metadata.get_component_storage(*component_id2)
                            {
                                AccessConflict::yes_if_true(fc.fid == *fragment_id)
                            } else {
                                AccessConflict::No
                            }
                        }
                        ComponentReferenceKind::Immutable
                        | ComponentReferenceKind::Discriminant => {
                            if let Some(Some(fc)) =
                                fragment_metadata.get_component_storage(*component_id2)
                            {
                                AccessConflict::yes_if_true(
                                    fc.fid == *fragment_id
                                        && matches!(reference_kind, ReferenceKind::Mutable),
                                )
                            } else {
                                AccessConflict::No
                            }
                        }
                    }
                }
                _ => other.conflicts(self, fragment_metadata),
            },
            AccessUnit::Component(component_id, component_reference_kind, entity_filter) => {
                match other {
                    AccessUnit::Component(
                        component_id2,
                        component_reference_kind2,
                        entity_filter2,
                    ) => match (component_reference_kind, component_reference_kind2) {
                        (ComponentReferenceKind::Discriminant, o) => AccessConflict::yes_if_true(
                            o == ComponentReferenceKind::DiscriminantMutable
                                && component_id2 == component_id
                                && entity_filter.share_path(entity_filter2),
                        ),
                        (_, ComponentReferenceKind::Discriminant) => AccessConflict::yes_if_true(
                            o == ComponentReferenceKind::DiscriminantMutable
                                && component_id2 == component_id
                                && entity_filter.share_path(entity_filter2),
                        ),
                        (ComponentReferenceKind::Immutable, ComponentReferenceKind::Immutable) => {
                            AccessConflict::No
                        }
                        (
                            ComponentReferenceKind::Mutable | ComponentReferenceKind::Immutable,
                            ComponentReferenceKind::Mutable | ComponentReferenceKind::Immutable,
                        ) => AccessConflict::yes_if_true(
                            *component_id == *component_id2
                                && entity_filter.share_path(entity_filter2),
                        ),
                        // one of these values is DiscriminantMutable, filters do not matter here,
                        // because all entities' components are influenced.
                        (f, s) => {
                            let test = |fid: FragmentId| {
                                if let Some(Some(fc)) =
                                    fragment_metadata.get_component_storage(*component_id2)
                                {
                                    if fid == fc.fid {
                                        return true;
                                    }
                                }

                                if o == ComponentReferenceKind::DiscriminantMutable
                                    && let Some(iter) =
                                        fragment_metadata.get_component_markers(*component_id2)
                                {
                                    for fc in iter {
                                        if fid == fc.fid {
                                            return true;
                                        }
                                    }
                                }

                                false
                            };

                            'ret: loop {
                                if let Some(Some(fc)) =
                                    fragment_metadata.get_component_storage(*component_id)
                                {
                                    if test(fc.fid) {
                                        break AccessConflict::Yes;
                                    }
                                }

                                if f == ComponentReferenceKind::DiscriminantMutable
                                    && let Some(iter) =
                                        fragment_metadata.get_component_markers(*component_id)
                                {
                                    for fc in iter {
                                        if test(fc.fid) {
                                            break 'ret AccessConflict::Yes;
                                        }
                                    }
                                }

                                break AccessConflict::No;
                            }
                        }
                    },
                    _ => other.conflicts(self, fragment_metadata),
                }
            }
        }
    }

    pub fn is_subset(
        &self,
        other: &Self,
        fragment_metadata: UnsafeFragmentComponentMetadataRef,
    ) -> bool {
        todo!()
    }
}

#[derive(Clone, Copy)]
pub enum ReferenceKind {
    Mutable,
    Immutable,
}

impl ReferenceKind {
    pub fn conflicts(self, other: Self) -> AccessConflict {
        AccessConflict::yes_if_true(matches!(
            (self, other),
            (ReferenceKind::Mutable, _) | (_, ReferenceKind::Mutable)
        ))
    }

    pub fn is_subset(self, other: Self) -> bool {
        matches!(
            (self, other),
            (_, ReferenceKind::Mutable) | (ReferenceKind::Immutable, ReferenceKind::Immutable)
        )
    }
}

pub enum ComponentReferenceKind {
    DiscriminantMutable,
    Mutable,
    Immutable,
    Discriminant,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct BinaryEntityFilter(usize);

impl BinaryEntityFilter {
    pub const fn new(id: usize) -> Option<Self> {
        let p = (id << 1) >> 1;
        (p == id).then(|| Self(id << 1))
    }

    pub const fn reverse(self) -> Self {
        Self(self.0 ^ 0b1)
    }

    pub const fn id(self) -> usize {
        self.0 >> 1
    }
}

pub struct EntityFilter(GorcRef<'static, [GorcRef<'static, SortedSet<[BinaryEntityFilter]>>]>);

impl EntityFilter {
    pub fn share_path(&self, other: &Self) -> bool {
        for path1 in self.0.iter() {
            for path2 in other.0.iter() {
                let mut iter1 = path1.iter().cloned().map(BinaryEntityFilter::reverse);
                let mut iter2 = path2.iter().cloned();

                loop {
                    let Some(v1) = iter1.clone().next() else {
                        return true;
                    };
                    let Some(v2) = iter2.clone().next() else {
                        return true;
                    };
                    if v1 == v2 {
                        break;
                    }
                    let id1 = v1.id();
                    let id2 = v2.id();
                    let minid = id1.min(id2);
                    if minid == id1 {
                        iter1.next();
                    }
                    if minid == id2 {
                        iter2.next();
                    }
                }
            }
        }
        false
    }
}
