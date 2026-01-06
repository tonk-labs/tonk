use dialog_common::ConditionalSend;
use futures_util::Stream;

/// NOTE: This type was adapted from https://github.com/Nullus157/async-compression/blob/main/src/unshared.rs
/// Original implementation licensed MIT/Apache 2
///
/// Wraps a type and only allows unique borrowing, the main usecase is to wrap a `!Sync` type and
/// implement `Sync` for it as this type blocks having multiple shared references to the inner
/// value.
///
/// # Safety
///
/// We must be careful when accessing `inner`, there must be no way to create a shared reference to
/// it from a shared reference to an `Exclusive`, as that would allow creating shared references on
/// multiple threads.
///
/// As an example deriving or implementing `Clone` is impossible, two threads could attempt to
/// clone a shared `Exclusive<T>` reference which would result in accessing the same inner value
/// concurrently.
#[repr(transparent)]
pub struct Exclusive<T>(T);

impl<T> Exclusive<T> {
    /// Initialize a new [Exclusive], wrapping the provided inner value
    pub fn new(inner: T) -> Self {
        Exclusive(inner)
    }

    /// Get a mutable (unique) reference to the inner value
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Safety: See comments on main docs for `Exclusive`
unsafe impl<T> Sync for Exclusive<T> {}
unsafe impl<T> Send for Exclusive<T> {}

impl<T> std::fmt::Debug for Exclusive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(core::any::type_name::<T>()).finish()
    }
}

/// Wrapper that implements [Stream] for any [Exclusive] that happens to wrap an
/// appropriately bounded [Stream]. This is useful for making a `!Sync` stream
/// into a `Sync` one in cases where we know it will not be shared by concurrent
/// actors.
///
/// Implementation note: we do not implement [Stream] directly on [Exclusive] as
/// an expression of hygiene; only mutable borrows of the inner value should be
/// possible in order to preserve the soundness of [Exclusive].
#[repr(transparent)]
pub struct ExclusiveStream<T>(Exclusive<T>)
where
    T: Stream + Unpin,
    for<'a> T::Item: ConditionalSend + 'a;

impl<T> ExclusiveStream<T>
where
    T: Stream + Unpin,
    for<'a> T::Item: ConditionalSend + 'a,
{
    /// Initialize a new [ExclusiveStream] wrapping a provided (presumably `!Sync`)
    /// [Stream]
    pub fn new(inner: T) -> Self {
        ExclusiveStream(Exclusive::new(inner))
    }
}

impl<T> Stream for ExclusiveStream<T>
where
    T: Stream + Unpin,
    for<'a> T::Item: ConditionalSend + 'a,
{
    type Item = T::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::pin!(self.get_mut().0.get_mut()).poll_next(cx)
    }
}
