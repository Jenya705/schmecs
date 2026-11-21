use std::{
    alloc::{Layout, alloc, dealloc, realloc},
    borrow::Borrow,
    hash::Hash,
    hint::cold_path,
    marker::PhantomData,
    num::NonZero,
    ptr::NonNull,
};

use hashbrown::{DefaultHashBuilder, HashMap};

use crate::{
    AllocError,
    access::{AccessSet, checked::AccessCheck},
    bundle::{BundleEdges, BundleId, UnsafeBundleComponent},
    component::{ComponentData, ComponentId, Components, LocalComponentId},
    debug::DebugVariable,
    entity::{Entities, EntityIndex},
    fragment::{FragmentComponents, FragmentId, StoredFragmentComponents},
    gorc::GorcRef,
    hash_map::TraitlessHashMapRawEntryBuilderMut,
    id_struct,
    nonmax::NonMaxU32,
    page::PageSparseSet,
    sparse::{ImmutableSparseSet, ImmutableSparseSetPtr, SparseSet},
    unsafe_cell,
};

id_struct!(pub TableId = NonMaxU32);
id_struct!(pub TableRow = u32);

pub struct Tables {
    tables: Vec<Table>,
    components2id: HashMap<FragmentComponents, TableId>,
    // This is basically (value -> index) mapping for (index -> value).
    col_indicies:
        HashMap<StoredFragmentComponents, ImmutableSparseSet<LocalComponentId, NonMaxU32>>,
    // As we have more fragments, more component ids won't be used by certain fragment hence storing
    // components sparsely is going to be much more expensive, by mapping each component id to its' local
    // id we basically reduce number of components for this fragment.
    //
    // NOTE: ordering of local component ids is the same as their corresponding original ids.
    // forall cid_1, cid_2 : cid_1 < cid_2 => lcid(cid_1) < lcid(cid_2)
    local_to_global: Vec<ComponentId>,
    // The idea behind using a paged sparse set is to decrease memory usage for each table, because
    // not every entity is actually stored in this fragment.
    //
    // The downside is that it adds another dereference, by default it could be:
    // Entity -> TableId, TableRow
    // In this case it is:
    // Entity -> Page -> TableId, TableRow
    //
    // By really packed fragments, this set actually increases memory usage by approximately
    // 1/EXPECTED.
    // Otherwise it packs EXPECTED longs into 1 usize (might save up to 1-EXPECTED/8 of memory on 64-bit
    // architecture).
    //
    // An entity doesn't have any location incase it possesses no components of these tables, it
    // slows iteration of such entities but it is paramount for memory because otherwise each
    // fragment would had to store every entity and not only those actually having corresponding
    // components.
    locations: PageSparseSet<
        EntityIndex,
        TableEntityLocation,
        { Entities::EXPECTED_PAGE_SIZE },
        { Entities::EXPECTED_PAGE_SIZE_LOG2 },
    >,
    bundle_edges: BundleEdges<Option<TableId>>,
}

const _: () = {
    const fn __as<T: Sync + Send>() {}
    __as::<Tables>();
};

#[derive(Debug, Clone, Copy)]
pub struct TableEntityLocation {
    pub table: TableId,
    pub row: TableRow,
}

const _TABLE_ENTITY_LOCATION_OPTION_PACKED: () = {
    assert!(
        std::mem::size_of::<TableEntityLocation>()
            == std::mem::size_of::<Option<TableEntityLocation>>()
    );
};

impl Tables {
    pub fn new() -> Self {
        Self {
            tables: vec![],
            components2id: HashMap::new(),
            col_indicies: HashMap::new(),
            local_to_global: vec![],
            locations: PageSparseSet::new(),
            bundle_edges: BundleEdges::new(),
        }
    }

    fn build_col_indicies_set(
        stored: &StoredFragmentComponents,
    ) -> ImmutableSparseSet<LocalComponentId, NonMaxU32> {
        let mut set = SparseSet::with_capacity(
            stored
                .stored()
                .last()
                .map(|v| v.id_usize() + 1)
                .unwrap_or(0),
        );
        for (i, &cid) in stored.stored().iter().enumerate() {
            // SAFETY: each `i` maps to exactly one unique LocalComponentId, there exists only u32::MAX - 1
            // local component ids because it is effectively NonMaxU32 hence it is safe to say that
            // i is less than u32::MAX.
            let i = unsafe { NonMaxU32::new_unchecked(i as u32) };
            // SAFETY: `stored` is sorted as per the contract of `FragmentComponents::new`, hence
            // the last element is the biggest key value, thus capacity is bigger than any possible
            // cid from `stored`, thus all ids are within the bounds of capacity.
            unsafe {
                set.insert_within_capacity(cid, i);
            }
        }
        set.into()
    }

    /// # Safety
    /// [`BundleId`] must correspond to a bundle which actually results in this transition.
    pub unsafe fn store_bundle_edge(
        &mut self,
        bundle_id: BundleId,
        from: Option<TableId>,
        to: Option<TableId>,
    ) {
        self.bundle_edges.insert(bundle_id, from, to);
    }

    pub fn get_bundle_edge(
        &self,
        bundle_id: BundleId,
        from: Option<TableId>,
    ) -> Option<Option<TableId>> {
        self.bundle_edges.get(bundle_id, from).cloned()
    }

    pub fn all_components(&self) -> std::iter::Cloned<std::slice::Iter<'_, ComponentId>> {
        self.local_to_global.iter().cloned()
    }

    pub fn get_location(&self, index: EntityIndex) -> Option<&TableEntityLocation> {
        self.locations.get(index)
    }

    pub fn get_table_id<'a, A>(
        &self,
        fragment_components: &FragmentComponents<A>,
    ) -> Option<TableId>
    where
        FragmentComponents<GorcRef<'a, [LocalComponentId]>>: Borrow<FragmentComponents<A>>,
        FragmentComponents<A>: Hash + Eq + 'a,
    {
        self.components2id.get(fragment_components).cloned()
    }

    /// # Safety
    /// If the entry is to be instantiated, the given factory method must be used.
    pub unsafe fn get_table_id_raw_entry_and_factory(
        &mut self,
    ) -> (
        TraitlessHashMapRawEntryBuilderMut<'_, FragmentComponents, TableId, DefaultHashBuilder>,
        impl FnMut(&FragmentComponents, &Components) -> TableId + '_,
    ) {
        (
            TraitlessHashMapRawEntryBuilderMut::new(&mut self.components2id),
            |fragment_components, components| {
                let stored_fragment_components =
                    StoredFragmentComponents(fragment_components.clone());

                // TODO: When calculating hash for FragmentComponents, one can also calculate hash
                // for StoredFragmentComponents.
                let col_indicies = self
                    .col_indicies
                    .entry(stored_fragment_components.clone())
                    .or_insert_with_key(|key| Self::build_col_indicies_set(key));

                // SAFETY:
                // - The caller's contract.
                // - `fragment_components` is Some as per the contract.
                // - `col_indicies` was instantiated using `Self::build_col_indicies_set`.
                // - `stored_fragment_components` was used to instantiate `col_indicies`.
                let table = unsafe {
                    Table::new(
                        fragment_components
                            .stored()
                            .iter()
                            .map(|id| {
                                self.local_to_global
                                    .get(id.id_usize())
                                    .cloned()
                                    .unwrap_unchecked()
                            })
                            .map(|id| GorcRef::clone(components.get(id).unwrap_unchecked())),
                        ImmutableSparseSetPtr::new(col_indicies),
                        stored_fragment_components,
                    )
                };

                let id = TableId::new(NonMaxU32::new(self.tables.len() as u32).unwrap());
                self.tables.push(table);

                id
            },
        )
    }

    /// # Safety
    /// - All components in `table_components` must exist in `components` and have local component
    /// ids within the [`Tables`].
    pub unsafe fn insert_or_get_table_id(
        &mut self,
        fragment_components: FragmentComponents,
        components: &Components,
    ) -> TableId {
        let (entry_builder, mut factory) = unsafe { self.get_table_id_raw_entry_and_factory() };

        let (_, entry) = entry_builder.from_key(&fragment_components);

        *entry
            .or_insert_with(|| {
                let id = factory(&fragment_components, components);
                (fragment_components, id)
            })
            .1
    }

    pub(crate) fn get_table_ptr_unchecked(&mut self, id: TableId) -> NonNull<Table> {
        unsafe { NonNull::from_mut(self.tables.get_unchecked_mut(id.id_usize())) }
    }

    pub fn get_table(&self, id: TableId) -> Option<&Table> {
        self.tables.get(id.id_usize())
    }

    pub fn get_table_mut(&mut self, id: TableId) -> Option<&mut Table> {
        self.tables.get_mut(id.id_usize())
    }

    /// # Safety
    /// - the given `cid` must be greater than the last added cid.
    pub unsafe fn add_cid(&mut self, cid: ComponentId) -> LocalComponentId {
        #[cfg(debug_assertions)]
        {
            if let Some(last) = self.local_to_global.last() {
                assert!(*last < cid);
            }
        }
        let id = self.local_to_global.len();
        self.local_to_global.push(cid);
        // SAFETY: length cannot be equal to u32::MAX, because there are only u32::MAX - 1
        // component ids.
        LocalComponentId::new(unsafe { NonMaxU32::new_unchecked(id as u32) })
    }

    pub fn get_global_cid(&self, cid: LocalComponentId) -> Option<ComponentId> {
        self.local_to_global.get(cid.id_usize()).cloned()
    }

    /// # Safety
    /// - `from_id` and `to_id` may not be equal.
    /// - `additional` has to output pointers to the valid objects for the given [`LocalComponentId`].
    /// - in case `from_id` is not [`Option::None`], entity with `index` must exist within the
    /// [`Table`] with id `from_id`, otherwise the entity may not exist within any [`Table`] of this [`Tables`].
    /// - `from_id` must exist within this [`Tables`] in case `from_id` is not [`Option::None`].
    /// - `to_id` must exist within this [`Tables`] in case `to_id` is not [`Option::None`].
    pub unsafe fn relocate_entity(
        &mut self,
        from_id: Option<TableId>,
        to_id: Option<TableId>,
        mut bundle: impl Iterator<Item = UnsafeBundleComponent>,
        index: EntityIndex,
    ) {
        match (from_id, to_id) {
            // SAFETY: `from_id` and `to_id` are never equal as per the contract.
            (None, None) => unsafe { std::hint::unreachable_unchecked() },
            (None, Some(to_id)) => {
                let table = unsafe { self.tables.get_unchecked_mut(to_id.id_usize()) };

                let row = TableRow::new(table.len as _);

                unsafe {
                    table.push_row(
                        // SAFETY: [`Bundle::next_component`] returns components in the sorted
                        // order and must return all components.
                        std::iter::from_fn(|| {
                            loop {
                                if let UnsafeBundleComponent::Add { ptr, .. } = bundle.next()? {
                                    return Some(ptr);
                                }
                            }
                        }),
                        index,
                    )
                };

                self.locations
                    .insert(index, TableEntityLocation { table: to_id, row });
            }
            (Some(from_id), None) => {
                let table = unsafe { self.tables.get_unchecked_mut(from_id.id_usize()) };
                let row = unsafe { self.locations.get(index).unwrap_unchecked() }.row;

                let swapped = unsafe { table.drop_row(row) };

                self.locations.remove(index);

                if let Some(swapped) = swapped {
                    self.locations.insert(
                        swapped,
                        TableEntityLocation {
                            table: from_id,
                            row,
                        },
                    );
                }
            }
            (Some(from_id), Some(to_id)) => {
                let [from, to] = unsafe {
                    self.tables
                        .get_disjoint_unchecked_mut([from_id.id_usize(), to_id.id_usize()])
                };

                let row = unsafe { self.locations.get(index).unwrap_unchecked() }.row;
                let new_row = TableRow::new(to.len as _);

                let mut next_cur = || -> Option<(LocalComponentId, NonNull<u8>)> {
                    loop {
                        let comp = bundle.next()?;
                        if let UnsafeBundleComponent::Add { component_id, ptr } = comp {
                            break Some((component_id, ptr));
                        }
                    }
                };

                let mut cur: Option<(LocalComponentId, NonNull<u8>)> = next_cur();

                let res = unsafe {
                    from.move_row(
                        to,
                        move |lcid| {
                            if let Some((slcid, ptr)) = cur
                                && slcid == lcid
                            {
                                cur = next_cur();
                                Some(ptr)
                            } else {
                                None
                            }
                        },
                        row,
                    )
                };

                self.locations.insert(
                    index,
                    TableEntityLocation {
                        table: to_id,
                        row: new_row,
                    },
                );

                if let Some(swapped) = res.swapped {
                    self.locations.insert(
                        swapped,
                        TableEntityLocation {
                            table: from_id,
                            row,
                        },
                    );
                }
            }
        }
    }
}

pub struct Table {
    cols: Box<[Column<GorcRef<'static, ComponentData>>]>,
    indicies: Column<ConstColumnData<EntityIndex>>,
    col_indicies: ImmutableSparseSetPtr<LocalComponentId, NonMaxU32>,
    components: StoredFragmentComponents,
    len: usize,
    cap: usize,
}

impl Drop for Table {
    fn drop(&mut self) {
        if self.cap != 0 {
            for col in self.cols.iter_mut() {
                // SAFETY:
                // - `cap` is not 0.
                // - `len` and `cap` are the same amongst all columns of this table.
                unsafe {
                    col.drop(self.len, self.cap);
                }
            }
            // SAFETY: see above.
            unsafe {
                self.indicies.drop(self.len, self.cap);
            }
        }
    }
}

pub struct TableRowMoveResult {
    pub new_row: TableRow,
    pub swapped: Option<EntityIndex>,
}

impl Table {
    /// # Safety
    /// - All component data yielded by `components` must live as long as this Table does.
    /// - `col_indicies` must be a valid mapping (cid -> column index) for this table.
    /// - `stored_fragment_components` must correspond to `col_indicies`.
    pub unsafe fn new<'a>(
        components: impl Iterator<Item = GorcRef<'static, ComponentData>>,
        col_indicies: ImmutableSparseSetPtr<LocalComponentId, NonMaxU32>,
        stored_fragment_components: StoredFragmentComponents,
    ) -> Self {
        let cols = Vec::from_iter(components.map(|data| Column::new(data))).into_boxed_slice();

        Self {
            cols,
            col_indicies,
            components: stored_fragment_components,
            indicies: Column::new(ConstColumnData::new()),
            len: 0,
            cap: 0,
        }
    }

    pub unsafe fn reserve(&mut self, elements: usize) {
        if self.cap == 0 {
            cold_path();
            for col in self.cols.iter_mut() {
                // SAFETY: `elements` is always a valid capacity as per the contract.
                unsafe {
                    // TODO: handle error
                    col.alloc(elements).unwrap();
                }
            }
            // SAFETY: see above.
            unsafe {
                self.indicies.alloc(elements).unwrap();
            }
            self.cap = 1;
        } else {
            let new_cap = (self.cap << 1).max(self.cap + elements);
            for col in self.cols.iter_mut() {
                // SAFETY:
                // - `self.cap` is the capacity of all columns.
                // - `new_cap` is greater than zero because `self.cap` is not 0.
                // - `new_cap` cannot be that big for the result to be greater than isize::MAX.
                unsafe {
                    // TODO: handle error
                    col.realloc(self.cap, new_cap).unwrap();
                }
            }
            // SAFETY: see above.
            unsafe {
                self.indicies.realloc(self.cap, new_cap).unwrap();
            }
            self.cap = new_cap;
        }
    }

    pub unsafe fn ensure_capacity(&mut self, elements: usize) {
        let needed = elements.saturating_sub(self.cap - self.len);
        if needed != 0 {
            unsafe {
                self.reserve(needed);
            }
        }
    }

    /// # Safety
    /// - `values` has to match the ordering of components within the table.
    /// - the underlying types of `values` must match the components.
    /// - the length of `values` must be equal to the number of components.
    /// - `values` cannot contain pointers into the uninitialized sections of the columns.
    /// - `values` must point to initialized objects (i.e., are safe to cast to the underlying type)
    pub unsafe fn push_row(
        &mut self,
        mut values: impl Iterator<Item = NonNull<u8>>,
        index: EntityIndex,
    ) {
        unsafe {
            self.ensure_capacity(1);
        }
        debug_assert!(self.len < self.cap);
        for (col, val) in self
            .cols
            .iter_mut()
            // SAFETY: `values` has the same length as the number of components.
            .map(|it| (it, unsafe { values.next().unwrap_unchecked() }))
        {
            // SAFETY:
            // - `self.len` is equal to the length.
            // - the current capacity is sufficient as per `ensure_capacity`.
            // - type matches as per the contract.
            // - no overlapping as per the contract.
            unsafe {
                col.forget_one_and_place_into(val, self.len);
            }
        }
        // SAFETY: see above and EntityIndex is the type of this column.
        unsafe {
            self.indicies
                .forget_one_and_place_into(NonNull::from_ref(&index).cast(), self.len);
        }
        self.len += 1;
    }

    pub unsafe fn drop_row(&mut self, row: TableRow) -> Option<EntityIndex> {
        unsafe fn __drop_row(
            mut remove: impl FnMut(&mut Column<GorcRef<ComponentData>>),
            mut remove_entity: impl FnMut(&mut Column<ConstColumnData<EntityIndex>>),
            this: &mut Table,
            _row: usize,
        ) {
            for col in this.cols.iter_mut() {
                remove(col);
            }
            remove_entity(&mut this.indicies);
        }

        let rowu = row.id_usize();
        let last = self.len - 1;

        let res = if rowu == last {
            cold_path();
            unsafe {
                __drop_row(
                    |col| col.remove_last(last),
                    |col| col.remove_last(last),
                    self,
                    rowu,
                )
            };
            None
        } else {
            unsafe {
                __drop_row(
                    |col| col.remove_one_and_place_last(last, rowu),
                    |col| col.remove_one_and_place_last(last, rowu),
                    self,
                    rowu,
                );
            }
            Some(*unsafe { self.indicies.get_typed::<EntityIndex>(rowu) })
        };

        self.len -= 1;

        res
    }

    /// # Safety
    /// - The given `row` must exist within the table.
    /// - `additional` must return some pointer for columns that don't exist in the `self` table but
    /// exist in the `other` table.
    pub unsafe fn move_row(
        &mut self,
        other: &mut Table,
        additional: impl FnMut(LocalComponentId) -> Option<NonNull<u8>>,
        row: TableRow,
    ) -> TableRowMoveResult {
        unsafe fn __move_row(
            mut forget: impl FnMut(&mut Column<GorcRef<ComponentData>>),
            forget_entity: impl FnOnce(&mut Column<ConstColumnData<EntityIndex>>),
            mut remove: impl FnMut(&mut Column<GorcRef<ComponentData>>),
            mut additional: impl FnMut(LocalComponentId) -> Option<NonNull<u8>>,
            this: &mut Table,
            other: &mut Table,
            this_row: usize,
        ) {
            let mut this_c = 0usize;
            let mut other_c = 0usize;

            while let Some(other_next) = other.components.stored().get(other_c).cloned() {
                if let Some(additional) = additional(other_next) {
                    unsafe {
                        other.cols[other_c].forget_one_and_place_into(additional, other.len);
                    }
                } else {
                    let mut first = true;
                    loop {
                        match this.components.stored().get(this_c).cloned() {
                            Some(this_next) if this_next < other_next => {
                                let this_col = &mut this.cols[this_c];
                                remove(this_col);
                            }
                            Some(this_next) if this_next == other_next => {
                                let this_col = &mut this.cols[this_c];
                                let ptr = unsafe { this_col.get(this_row) };
                                unsafe {
                                    other.cols[other_c].forget_one_and_place_into(ptr, other.len);
                                }
                                forget(this_col);
                            }
                            _ if !first => {
                                break;
                            }
                            _ => {
                                // SAFETY: `additional` must return some pointer for columns that
                                // don't exist in `this` table, which means that if this_next is greater than
                                // other_next (which is the case here) then additional would had to
                                // return some pointer (because `other_next` doesn't exist within
                                // `this` table).
                                unsafe {
                                    std::hint::unreachable_unchecked();
                                }
                            }
                        }
                        first = false;
                        this_c += 1;
                    }
                }
                other_c += 1;
            }

            while let Some(_) = this.components.stored().get(this_c).cloned() {
                remove(&mut this.cols[this_c]);
                this_c += 1;
            }

            let index = unsafe { this.indicies.get_typed::<EntityIndex>(this_row) };
            unsafe {
                other
                    .indicies
                    .forget_one_and_place_into(NonNull::from_ref(index).cast(), other.len);
            }
            forget_entity(&mut this.indicies);
        }

        let rowu = row.id_usize();
        let last = self.len - 1;

        unsafe {
            other.ensure_capacity(1);
        }

        let swapped = if rowu == last {
            cold_path();
            unsafe {
                __move_row(
                    |col| col.forget_last(rowu),
                    |col| col.forget_last(rowu),
                    |col| col.remove_last(rowu),
                    additional,
                    self,
                    other,
                    rowu,
                );
            }
            None
        } else {
            unsafe {
                __move_row(
                    |col| col.forget_one_and_place_last(last, rowu),
                    |col| col.forget_one_and_place_last(last, rowu),
                    |col| col.remove_one_and_place_last(last, rowu),
                    additional,
                    self,
                    other,
                    rowu,
                );
            }
            Some(*unsafe { self.indicies.get_typed::<EntityIndex>(rowu) })
        };

        let result = TableRowMoveResult {
            new_row: TableRow::new(other.len as u32),
            swapped,
        };

        self.len -= 1;
        other.len += 1;

        result
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn cap(&self) -> usize {
        self.cap
    }

    pub const fn stored_fragment_components(&self) -> &StoredFragmentComponents {
        &self.components
    }
}

/// # Safety
/// - `layout` must return a layout for an object which can be safely dropped with `drop`.
pub unsafe trait ColumnData {
    fn layout(&self) -> Layout;

    /// # Safety
    /// The given `ptr` must correspond to the object with the right type.
    unsafe fn drop(&self, ptr: *mut u8);

    fn must_be_dropped(&self) -> bool;
}

unsafe impl<'a> ColumnData for GorcRef<'a, ComponentData> {
    fn layout(&self) -> Layout {
        self.layout
    }

    unsafe fn drop(&self, ptr: *mut u8) {
        if let Some(drop) = self.drop {
            // SAFETY: The caller's contract.
            unsafe {
                drop(ptr);
            }
        }
    }

    fn must_be_dropped(&self) -> bool {
        self.drop.is_some()
    }
}

pub struct ConstColumnData<T>(PhantomData<fn() -> T>);

impl<T> ConstColumnData<T> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

unsafe impl<T> ColumnData for ConstColumnData<T> {
    fn layout(&self) -> Layout {
        Layout::new::<T>()
    }

    unsafe fn drop(&self, ptr: *mut u8) {
        if self.must_be_dropped() {
            // SAFETY: The caller's contract
            unsafe { std::ptr::drop_in_place(ptr.cast::<T>()) }
        }
    }

    fn must_be_dropped(&self) -> bool {
        std::mem::needs_drop::<T>()
    }
}

pub struct Column<D> {
    ptr: NonNull<u8>,
    data: D,
    len: DebugVariable<usize>,
    cap: DebugVariable<usize>,
}

// TODO: SAFETY COMMENTS (Components are always Sync and Send -> propagate this to ColumnData and ComponentData)
unsafe impl<D> Sync for Column<D> where D: Sync {}
unsafe impl<D> Send for Column<D> where D: Send {}

impl<D> Column<D>
where
    D: ColumnData,
{
    #[inline(always)]
    fn alloc_layout(&self, cap: usize) -> Layout {
        self.data.layout().repeat(cap).unwrap().0
    }

    #[inline(always)]
    fn element_size(&self) -> usize {
        self.data.layout().size()
    }

    /// # Safety
    /// - `len` must match the respective length of the container.
    unsafe fn ptr_iter(&self, len: usize) -> impl Iterator<Item = NonNull<u8>> {
        struct Iter {
            ptr: NonNull<u8>,
            off: usize,
            len: usize,
        }
        impl Iterator for Iter {
            type Item = NonNull<u8>;

            fn next(&mut self) -> Option<Self::Item> {
                if self.len == 0 {
                    return None;
                }
                let res = Some(self.ptr);
                // SAFETY: `ptr` has to have len*off allocated bytes as per the contract.
                self.ptr = unsafe { self.ptr.add(self.off) };
                self.len -= 1;
                res
            }
        }
        Iter {
            ptr: self.ptr,
            off: self.element_size(),
            len,
        }
    }

    unsafe fn ptr_iter_backwards(&self, len: usize) -> impl Iterator<Item = NonNull<u8>> {
        struct Iter {
            ptr: NonNull<u8>,
            off: usize,
            len: usize,
        }
        impl Iterator for Iter {
            type Item = NonNull<u8>;

            fn next(&mut self) -> Option<Self::Item> {
                if self.len == 0 {
                    return None;
                }
                self.ptr = unsafe { self.ptr.sub(self.off) };
                self.len -= 1;
                // SAFETY: `ptr` has to have len*off allocated bytes as per the contract.
                Some(self.ptr)
            }
        }
        let off = self.element_size();
        Iter {
            ptr: unsafe { self.ptr.byte_add(off * len) },
            off,
            len,
        }
    }

    /// # Safety
    /// - `T` must be the underlying or the equivalent type of this column.
    /// - `len` must match the respective length of the container.
    pub unsafe fn typed_iter<'a, T: 'a>(&'a self, len: usize) -> impl Iterator<Item = &'a T> {
        // SAFETY: `len` matches the length of this container as per the contract.
        unsafe { self.ptr_iter(len) }
            // SAFETY: T is the equivalent type as per the contract.
            .map(|ptr| unsafe { ptr.cast::<T>().as_ref() })
    }

    /// # Safety
    /// - `T` must be the underlying or the equivalent type of this column.
    /// - `len` must match the respective length of the container.
    pub unsafe fn typed_mut_iter<'a, T: 'a>(
        &'a mut self,
        len: usize,
    ) -> impl Iterator<Item = &'a mut T> {
        // SAFETY: `len` matches the length of this container as per the contract.
        unsafe { self.ptr_iter(len) }
            // SAFETY: T is the equivalent type as per the contract.
            .map(|ptr| unsafe { ptr.cast::<T>().as_mut() })
    }

    pub unsafe fn typed_iter_backwards<'a, T: 'a>(
        &'a self,
        len: usize,
    ) -> impl Iterator<Item = &'a T> {
        unsafe { self.ptr_iter_backwards(len) }.map(|ptr| unsafe { ptr.cast::<T>().as_ref() })
    }

    #[inline(always)]
    unsafe fn get(&self, i: usize) -> NonNull<u8> {
        unsafe { self.ptr.add(i * self.element_size()) }
    }

    /// # Safety
    /// - `T` must be the underlying or the equivalent type of this column.
    /// - `i` is within the bounds.
    pub unsafe fn get_typed<T>(&self, i: usize) -> &T {
        // SAFETY: The type is correct and `i` is within the bounds.
        unsafe { self.get(i).cast().as_ref() }
    }

    pub fn new(data: D) -> Self {
        Self {
            // SAFETY: 0 is not a power of 2.
            ptr: NonNull::without_provenance(unsafe {
                NonZero::new_unchecked(data.layout().align())
            }),
            data,
            len: DebugVariable::new(0),
            cap: DebugVariable::new(0),
        }
    }

    /// # Safety
    /// - `cap` may not be 0.
    /// - `len` and `cap` must match the respective length and capacity of the container and the
    /// allocation
    pub unsafe fn drop(&mut self, len: usize, cap: usize) {
        self.len.assert_eq(len);
        self.cap.assert_eq(cap);
        if self.data.must_be_dropped() {
            // SAFETY: `len` matches the container's length.
            for ptr in unsafe { self.ptr_iter(len) } {
                // SAFETY: `ptr` references the needed type.
                unsafe {
                    self.data.drop(ptr.as_ptr());
                }
            }
        }
        // SAFETY:
        // - `cap` matches the allocation.
        // - `cap` is not zero which would otherwise indicate no allocation.
        unsafe {
            dealloc(self.ptr.as_ptr(), self.alloc_layout(cap));
        }
        self.len = DebugVariable::new(0);
        self.cap = DebugVariable::new(0);
    }

    /// # Safety
    /// - `new_cap` is greater than zero and the result of multiplication of the size of the object
    /// stored within the column by `new_cap` rounded up to the nearest multiple of the object's
    /// alignment does not overflow isize.
    ///
    /// # Leaks
    /// Calling this method twice without clearing the column will result in a memory leak.
    pub unsafe fn alloc(&mut self, new_cap: usize) -> Result<(), AllocError> {
        debug_assert!(new_cap > 0);
        self.cap.assert_eq(0);
        let new_layout = self.alloc_layout(new_cap);
        let ptr = unsafe { alloc(new_layout) };
        self.ptr = NonNull::new(ptr).ok_or(AllocError)?;
        self.cap = DebugVariable::new(new_cap);
        Ok(())
    }

    /// # Safety
    /// - `old_cap` must match the current allocation length and be greater than zero.
    /// - `new_cap` is greater than zero and the result of multiplication of the size of the object
    /// stored within the column by `new_cap` rounded up to the nearest multiple of the object's
    /// alignment does not overflow isize.
    pub unsafe fn realloc(&mut self, old_cap: usize, new_cap: usize) -> Result<(), AllocError> {
        self.cap.assert_eq(old_cap);
        debug_assert!(new_cap > 0);
        let old_layout = self.alloc_layout(old_cap);
        let new_layout = self.alloc_layout(new_cap);
        // SAFETY:
        // - `ptr` could only be retrieved from alloc or realloc hence it was allocated with the
        // global allocator.
        // - `old_cap` matches the current allocation (as per the contract) hence the layouts match.
        // - `new_cap` is greater than zero and the resulting size is also valid (as per the
        // contract).
        let ptr = unsafe { realloc(self.ptr.as_ptr(), old_layout, new_layout.size()) };
        self.ptr = NonNull::new(ptr).ok_or(AllocError)?;
        self.cap = DebugVariable::new(new_cap);
        Ok(())
    }

    /// # Safety
    /// - `to` must be within the bounds of the capacity.
    /// - `from` and `to` must be within bounds and not be equal.
    /// - `from` must be the last element.
    pub unsafe fn forget_one_and_place_last(&mut self, from: usize, to: usize) {
        self.len.assert_eq(from + 1);
        self.len.within_range(to);
        let size = self.element_size();
        // SAFETY: pointers are within the bounds and do not overlap because `from` and `to` are
        // within the bounds and are not equal.
        unsafe {
            std::ptr::copy_nonoverlapping(self.get(from).as_ptr(), self.get(to).as_ptr(), size);
        }
        self.len = self.len.map(|len| len - 1);
    }

    /// # Safety
    /// - `to` must be within the bounds or be equal to the length.
    /// - `to` must be within the bounds of the capacity.
    /// - `obj` must reference the underlying column's object.
    /// - `obj` may not be the pointer to `to`.
    pub unsafe fn forget_one_and_place_into(&mut self, obj: NonNull<u8>, to: usize) {
        self.len.assert(|&val| val >= to);
        self.cap.within_range(to);
        let size = self.element_size();
        // SAFETY: See the contract.
        unsafe {
            std::ptr::copy_nonoverlapping(obj.as_ptr(), self.get(to).as_ptr(), size);
        }
        self.len = self.len.map(|v| v.max(to + 1));
    }

    /// # Safety
    /// - `last` must be the last element.
    pub unsafe fn remove_last(&mut self, last: usize) {
        // SAFETY: The caller's contract.
        unsafe {
            self.forget_last(last);
        }
        // SAFETY: the underlying type matches.
        unsafe {
            self.data.drop(self.get(last).as_ptr());
        }
    }

    /// # Safety
    /// - `last` must be the last element.
    #[inline(always)]
    pub unsafe fn forget_last(&mut self, last: usize) {
        // NOTE: NOOP in release mode.
        self.len.assert_eq(last + 1);
        self.len = self.len.map(|v| v - 1);
    }

    /// # Safety
    /// - `from` and `to` must be within the bounds and not be equal.
    /// - `from` must be the last element.
    pub unsafe fn remove_one_and_place_last(&mut self, from: usize, to: usize) {
        self.len.within_range(to);
        // SAFETY: the underlying type matches and the caller's contract.
        unsafe {
            self.data.drop(self.get(to).as_ptr());
        }
        // SAFETY: the same contract.
        unsafe {
            self.forget_one_and_place_last(from, to);
        }
    }
}

unsafe_cell!(pub UnsafeTableCell<'a> = Table, (&'a AccessSet<'a>, FragmentId, TableId));

impl<'a, AC> UnsafeTableCell<'a, AC>
where
    AC: AccessCheck<Inner = (&'a AccessSet<'a>, FragmentId, TableId)>,
{
    pub unsafe fn column(
        &self,
        id: usize,
    ) -> UnsafeTableColumnCell<
        'a,
        GorcRef<'static, ComponentData>,
        AC::Map<(&'a AccessSet<'a>, FragmentId, TableId, usize)>,
    > {
        unsafe {
            UnsafeTableColumnCell::new(
                NonNull::from_mut(self.ptr().as_mut().cols.get_unchecked_mut(id)),
                self.access().ref_map(|v| (v.0, v.1, v.2, id)),
            )
        }
    }

    pub unsafe fn table_ref(&self) -> &Table {
        unsafe { self.ptr().as_ref() }
    }

    pub unsafe fn table_mut(&mut self) -> &mut Table {
        unsafe { self.ptr().as_mut() }
    }

    // NOTE: The methods marked as safe return something that cannot change in case no exclusive
    // access exists. When an exclusive access exists, this cell must be it, because otherwise it is
    // a violation.
    pub fn indicies(
        &self,
    ) -> UnsafeTableColumnCell<
        'a,
        ConstColumnData<EntityIndex>,
        AC::Map<(&'a AccessSet<'a>, FragmentId, TableId, usize)>,
    > {
        unsafe {
            UnsafeTableColumnCell::new(
                NonNull::from_mut(&mut self.ptr().as_mut().indicies),
                self.access().ref_map(|v| (v.0, v.1, v.2, usize::MAX)),
            )
        }
    }

    pub fn stored_fragment_components(&self) -> &StoredFragmentComponents {
        &unsafe { self.ptr().as_ref() }.stored_fragment_components()
    }

    pub fn col_indicies_ref(&self) -> &ImmutableSparseSetPtr<LocalComponentId, NonMaxU32> {
        &unsafe { self.ptr().as_ref() }.col_indicies
    }

    pub fn len(&self) -> usize {
        unsafe { self.ptr().as_ref() }.len
    }

    pub fn cap(&self) -> usize {
        unsafe { self.ptr().as_ref() }.cap
    }
}

unsafe_cell!(pub UnsafeTableColumnCell<'a, D> = Column<D>, (&'a AccessSet<'a>, FragmentId, TableId, usize));

impl<'a, D, AC> UnsafeTableColumnCell<'a, D, AC>
where
    D: ColumnData,
    AC: AccessCheck<Inner = (&'a AccessSet<'a>, FragmentId, TableId, usize)>,
{
    unsafe fn __ref(&self) -> &Column<D> {
        unsafe { self.ptr().as_ref() }
    }

    unsafe fn __mut(&mut self) -> &mut Column<D> {
        unsafe { self.ptr().as_mut() }
    }

    /// # Safety
    /// - `row` must be within bounds.
    pub unsafe fn single_ptr(&self, row: TableRow) -> NonNull<u8> {
        unsafe { self.__ref().get(row.id_usize()) }
    }

    pub fn begin(&self) -> NonNull<u8> {
        unsafe { self.__ref().ptr }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        marker::PhantomData,
        mem::ManuallyDrop,
        ptr::NonNull,
        sync::atomic::{AtomicU64, Ordering::SeqCst},
    };

    use crate::{
        bundle::UnsafeBundleComponent,
        component::{ComponentData, Components, LocalComponentId},
        entity::EntityIndex,
        fragment::{FragmentComponents, StoredFragmentComponents},
        gorc::GorcRef,
        nonmax::NonMaxU32,
        slice::Partitioned,
        sparse::{ImmutableSparseSet, ImmutableSparseSetPtr, SparseSet},
        storage::table::{Table, TableRow, Tables},
        world::ZERO_WORLD_ID,
    };

    #[derive(PartialEq, Eq, Debug)]
    pub struct A(pub i32);
    #[derive(PartialEq, Eq, Debug)]
    pub struct B(pub u64);

    macro_rules! c_struct {
        () => {
            #[derive(PartialEq, Eq, Debug)]
            pub struct C(pub String, PhantomData<()>);

            static C_COUNTER: AtomicU64 = AtomicU64::new(0);

            impl Drop for C {
                fn drop(&mut self) {
                    C_COUNTER.fetch_sub(1, SeqCst);
                }
            }

            impl C {
                pub fn new(val: String) -> Self {
                    C_COUNTER.fetch_add(1, SeqCst);
                    Self(val, PhantomData)
                }
            }
        };
    }

    #[test]
    fn table_test() {
        const A_ID: LocalComponentId = LocalComponentId::new(NonMaxU32::new(0).unwrap());
        const C_ID: LocalComponentId = LocalComponentId::new(NonMaxU32::new(1).unwrap());

        c_struct!();

        let mut indicies = SparseSet::new();
        indicies.insert(A_ID, NonMaxU32::new(0).unwrap());
        indicies.insert(C_ID, NonMaxU32::new(1).unwrap());
        let indicies: ImmutableSparseSet<_, _> = indicies.into();

        let mut table = unsafe {
            Table::new(
                [ComponentData::new::<A>(), ComponentData::new::<C>()]
                    .map(|v| GorcRef::from_box(Box::new(v)))
                    .into_iter(),
                ImmutableSparseSetPtr::new(&indicies),
                StoredFragmentComponents(FragmentComponents(
                    Partitioned::new([2], GorcRef::from_ref([A_ID, C_ID].as_slice())).unwrap(),
                )),
            )
        };

        let values = [
            (A(0), C::new("zero".into())),
            (A(1), C::new("one".into())),
            (A(2), C::new("two".into())),
        ];

        assert_eq!(C_COUNTER.load(SeqCst) as usize, values.len());

        for (i, values) in values.map(|v| ManuallyDrop::new(v)).into_iter().enumerate() {
            let i = EntityIndex::new(NonMaxU32::new(i as _).unwrap());
            unsafe {
                table.push_row(
                    [
                        NonNull::from_ref(&values.0).cast::<u8>(),
                        NonNull::from_ref(&values.1).cast(),
                    ]
                    .into_iter(),
                    i,
                );
            }
        }

        for (i, a) in unsafe { table.cols[0].typed_iter::<A>(table.len()) }.enumerate() {
            assert_eq!(a.0, i as i32);
        }

        for (i, c) in unsafe { table.cols[1].typed_iter::<C>(table.len()) }.enumerate() {
            assert_eq!(c.0, ["zero", "one", "two"][i]);
        }

        unsafe { table.drop_row(TableRow::new(0)) };

        assert_eq!(C_COUNTER.load(SeqCst), 2);

        for (i, a) in unsafe { table.cols[0].typed_iter::<A>(table.len()) }.enumerate() {
            assert_eq!(a.0, [2, 1][i]);
        }

        for (i, c) in unsafe { table.cols[1].typed_iter::<C>(table.len()) }.enumerate() {
            assert_eq!(c.0, ["two", "one"][i]);
        }

        drop(table);

        assert_eq!(C_COUNTER.load(SeqCst), 0);
    }

    #[test]
    fn tables_test() {
        let mut tables = Tables::new();
        let mut components = Components::new(ZERO_WORLD_ID);

        c_struct!();

        macro_rules! new_comp {
            ($ty: ty) => {
                components
                    .push(GorcRef::from_box(Box::new(ComponentData::new::<$ty>())))
                    .unwrap()
            };
        }

        let a_id = new_comp!(A);
        let b_id = new_comp!(B);
        let c_id = new_comp!(C);

        let a_lid = unsafe { tables.add_cid(a_id) };
        let b_lid = unsafe { tables.add_cid(b_id) };
        let c_lid = unsafe { tables.add_cid(c_id) };

        assert_eq!(a_lid.id_u32(), 0);
        assert_eq!(b_lid.id_u32(), 1);
        assert_eq!(c_lid.id_u32(), 2);

        macro_rules! new_table {
            ($comps: expr) => {
                unsafe {
                    tables.insert_or_get_table_id(
                        FragmentComponents(
                            Partitioned::new(
                                [($comps).len()],
                                GorcRef::from_box($comps.into_boxed_slice()),
                            )
                            .unwrap(),
                        ),
                        &components,
                    )
                }
            };
        }

        let a_table = new_table!(vec![a_lid]);
        let ab_table = new_table!(vec![a_lid, b_lid]);
        let ac_table = new_table!(vec![a_lid, c_lid]);
        let abc_table = new_table!(vec![a_lid, b_lid, c_lid]);

        assert_eq!(a_table.id_u32(), 0);
        assert_eq!(ab_table.id_u32(), 1);
        assert_eq!(ac_table.id_u32(), 2);
        assert_eq!(abc_table.id_u32(), 3);

        let to_add = A(10);

        let zeroth = EntityIndex::new(NonMaxU32::new(0).unwrap());

        unsafe {
            tables.relocate_entity(
                None,
                Some(a_table),
                [UnsafeBundleComponent::Add {
                    component_id: a_lid,
                    ptr: NonNull::from_ref(&to_add).cast(),
                }]
                .into_iter(),
                zeroth,
            )
        };

        let table = tables.get_table_mut(a_table).unwrap();

        assert!(
            [A(10)]
                .iter()
                .eq(unsafe { table.cols[0].typed_iter::<A>(1) })
        );

        assert_eq!(C_COUNTER.load(SeqCst), 0);

        let to_add = C::new("c".into());

        assert_eq!(C_COUNTER.load(SeqCst), 1);

        unsafe {
            tables.relocate_entity(
                Some(a_table),
                Some(ac_table),
                [UnsafeBundleComponent::Add {
                    component_id: c_lid,
                    ptr: NonNull::from_ref(&to_add).cast(),
                }]
                .into_iter(),
                zeroth,
            );
        }

        let table = tables.get_table_mut(ac_table).unwrap();

        assert!(
            [A(10)]
                .iter()
                .eq(unsafe { table.cols[0].typed_iter::<A>(1) })
        );

        assert!(
            [&to_add]
                .into_iter()
                .eq(unsafe { table.cols[1].typed_iter::<C>(1) })
        );

        std::mem::forget(to_add);

        let to_add = B(32);

        assert_eq!(C_COUNTER.load(SeqCst), 1);

        unsafe {
            tables.relocate_entity(
                Some(ac_table),
                Some(ab_table),
                [UnsafeBundleComponent::Add {
                    component_id: b_lid,
                    ptr: NonNull::from_ref(&to_add).cast(),
                }]
                .into_iter(),
                zeroth,
            );
        }

        assert_eq!(C_COUNTER.load(SeqCst), 0);

        let table = tables.get_table_mut(ab_table).unwrap();
        assert!(
            [A(10)]
                .iter()
                .eq(unsafe { table.cols[0].typed_iter::<A>(1) })
        );
        assert!(
            [B(32)]
                .iter()
                .eq(unsafe { table.cols[1].typed_iter::<B>(1) })
        );
        drop(tables);

        assert_eq!(C_COUNTER.load(SeqCst), 0);
    }
}
