use futures::Future;

use std::{
  fmt::Debug,
  time::Duration
};

pub async fn retry<F, Fut, T, E> (mut f: F, delay: Duration, count: usize, context: &str) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: Debug
{
    assert!(count > 0);
    let count = count - 1;

    for i in 0..count {
        match f().await {
            Ok(x) => return Ok(x),
            Err(e) => {
                log::debug!("{context} failed - retrying ({i}): {e:?}");
                async_std::task::sleep(delay).await;
            }
        }
    }

    let res = f().await;
    if let Err(ref e) = res {
        log::error!("{context} failed - aborting: {e:?}");
    }
    res
}
