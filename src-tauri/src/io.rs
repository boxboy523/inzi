use std::fmt::Debug;

pub struct IO<T>(T);

impl<T> IO<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    pub fn init<U, F>(f: F) -> IO<U>
    where
        F: FnOnce() -> U,
    {
        IO(f())
    }

    pub fn raw(self) -> T {
        self.0
    }

    pub fn init_and_then<U, F, E>(f: F) -> Result<IO<U>, E>
    where
        F: FnOnce() -> Result<U, E>,
    {
        f().map(IO::new)
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> IO<U> {
        IO(f(self.0))
    }

    pub fn and_then<U, F, E>(self, f: F) -> Result<IO<U>, E>
    where
        F: FnOnce(T) -> Result<U, E>,
    {
        f(self.0).map(IO::new)
    }

    pub fn flat_map<U, F: FnOnce(T) -> IO<U>>(self, f: F) -> IO<U> {
        f(self.0)
    }

    pub fn consume<F: FnOnce(T)>(self, f: F) {
        f(self.0);
    }

    pub fn consume_and_then<F, E>(self, f: F) -> Result<(), E>
    where
        F: FnOnce(T) -> Result<(), E>,
    {
        f(self.0)
    }

    pub fn as_ref(&self) -> IO<&T> {
        IO(&self.0)
    }

    pub fn as_mut(&mut self) -> IO<&mut T> {
        IO(&mut self.0)
    }
}

impl<T: Clone> Clone for IO<T> {
    fn clone(&self) -> Self {
        IO(self.0.clone())
    }
}

impl<T> Debug for IO<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let type_name = std::any::type_name::<T>();
        let struct_name = format!("IO<{}>", type_name);
        f.debug_tuple(&struct_name).field(&"<Secret>").finish()
    }
}

impl<T: Copy> Copy for IO<T> {}

impl<T: Send + 'static> IO<T> {
    pub fn consume_and_spawn<F, Fut>(self, async_f: F)
    where
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(async move {
            async_f(self.0).await;
        });
    }

    pub async fn consume_and_wait<F, Fut>(self, async_f: F)
    where
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        async_f(self.0).await;
    }

    pub async fn async_init<F, Fut, U>(async_f: F) -> IO<U>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = U> + Send + 'static,
    {
        let result = async_f().await;
        IO(result)
    }

    pub async fn async_init_and_then<F, Fut, U, E>(async_f: F) -> Result<IO<U>, E>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<U, E>> + Send + 'static,
    {
        let result = async_f().await?;
        Ok(IO(result))
    }

    pub async fn async_map<U, F, Fut>(self, async_f: F) -> IO<U>
    where
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = U> + Send + 'static,
    {
        let result = async_f(self.0).await;
        IO(result)
    }

    pub async fn async_and_then<U, F, Fut, E>(self, async_f: F) -> Result<IO<U>, E>
    where
        F: FnOnce(T) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<U, E>> + Send + 'static,
    {
        let result = async_f(self.0).await?;
        Ok(IO(result))
    }
}

impl<'a, T> From<&'a IO<T>> for IO<&'a T> {
    fn from(io: &'a IO<T>) -> Self {
        IO(&io.0)
    }
}

pub trait Unzip {
    type Output;
    fn unzip(self) -> Self::Output;
}

macro_rules! impl_unzip {
    ($($($name:ident)+),+) => {
        $(
            impl<$($name),+> Unzip for IO<($($name,)+)> {
                type Output = ($(IO<$name>,)+);
                #[allow(non_snake_case)]
                fn unzip(self) -> Self::Output {
                    let ($($name,)+) = self.0;
                    ($(IO($name),)+)
                }
            }
        )+
    }
}

impl_unzip!(
    A B,
    A B C,
    A B C D,
    A B C D E,
    A B C D E F
);

pub trait Join {
    type Output;
    fn join(self) -> Self::Output;
}

macro_rules! impl_join {
    ($($($name:ident)+),+) => {
        $(
            impl<$($name),+> Join for ($(IO<$name>,)+) {
                type Output = IO<($($name,)+)>;

                #[allow(non_snake_case)]
                fn join(self) -> Self::Output {
                    let ($($name,)+) = self;
                    IO(($($name.0,)+))
                }
            }
        )+
    }
}

impl_join!(
    A B,
    A B C,
    A B C D,
    A B C D E,
    A B C D E F
);
