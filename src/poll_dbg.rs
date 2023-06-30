use std::{
    pin::Pin, 
    sync::{
        Arc,
        mpsc::{channel, Sender}
    }
};

use async_std::task::Task;
use futures::{Future, lock::Mutex};
use once_cell::sync::Lazy;

enum Status {
    Unknown,
    Pending,
    Ready
}

struct Msg {
    task: Task,
    context: Arc<String>,
    status: Status
}

static SENDER: Lazy<Mutex<Sender<Msg>>> = Lazy::new(|| {
        let (tx, rx) = channel();
        
        std::thread::spawn(move || {
            while let Ok(Msg { task, context, status }) = rx.recv() {
                let name = task.name().unwrap_or("<unknown>");

                match status {
                    Status::Unknown => log::trace!("{name}: poll: {context} was polled"),
                    Status::Pending => log::trace!("{name}: poll: {context} returned Poll::Pending"),
                    Status::Ready => log::trace!("{name}: poll: {context} returned Poll::Ready(..)"),
                }
            }
        });

        Mutex::new(tx)
    });

pub struct PollDbg<T> {
    context: Arc<String>,
    inner: Pin<Box<T>>,
    sender: Sender<Msg>
}

impl<T> PollDbg<T> {
    pub async fn new(inner: T, context: impl ToString) -> Self {
        Self {
            context: Arc::new(context.to_string()),
            inner: Box::pin(inner),
            sender: SENDER.lock().await.clone()
        }
    }
}

impl<F, T> Future for PollDbg<F>
where
    F: Future<Output = T>
{
    type Output =  T;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        use std::task::Poll;

        fn log<T> (this: &PollDbg<T>, status: Status) {
            this.sender.send(Msg {
                task: async_std::task::current(),
                context: this.context.clone(),
                status
            }).unwrap()
        }

        let this = self.get_mut();

        log(this, Status::Unknown);
        
        let poll = this.inner.as_mut().poll(cx);

        match poll {
            Poll::Pending => log(this, Status::Pending),
            Poll::Ready(_) => log(this, Status::Ready)
        }

        poll
    }
}

