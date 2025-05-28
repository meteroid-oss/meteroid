Additional macros for the Meteroid store crate.


### with_conn_delegate

Small macro to generate a proxy interface & impl for the #delegated method(s), to allow using them with or without providing a conn.
You are left with implementing the non-delegated methods + the with_conn method !

```rust
#[with_conn_delegate]
pub trait ItemInterface {
    #[delegated]
    async fn get_item(
        &self,
        item_id: Itemid,
    ) -> StoreResult<Item>;

    async fn insert_item(
        &self,
        item: ItemNew,
    ) -> StoreResult<()>;
}
// then you just implement
impl Store for ItemInterface {
    async fn get_item_with_conn(
      &self,
      conn: &mut PgConn,
      item_id: Itemid,
    ) -> StoreResult<Item> {...}

    async fn insert_item(
      &self,
      item: ItemNew,
    ) -> StoreResult<()> {...}
}

// Then both are available:
// store.get_item_with_conn(&mut conn, item_id).await;
// or 
// store.get_item(item_id).await; 
```
