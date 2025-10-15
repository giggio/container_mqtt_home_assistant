macro_rules! cast {
    ($target: expr, $pat: path) => {{
        if let $pat(a) = $target {
            // #1
            a
        } else {
            panic!("mismatch variant when cast to {}", stringify!($pat)); // #2
        }
    }};
}

// from https://github.com/rustonaut/common_macros/blob/61421b7e56ad2d6d0c5c1df6c22afa2efb4764b7/src/lib.rs#L44
#[macro_export]
macro_rules! const_expr_count {
    () => (0);
    ($e:expr) => (1);
    ($e:expr; $($other_e:expr);*) => ({
        1 $(+ $crate::const_expr_count!($other_e) )*
    });

    ($e:expr; $($other_e:expr);* ; ) => (
        $crate::const_expr_count! { $e; $($other_e);* }
    );
}

// from https://github.com/rustonaut/common_macros/blob/61421b7e56ad2d6d0c5c1df6c22afa2efb4764b7/src/lib.rs#L92
#[macro_export]
macro_rules! hashmap {
    (with $map:expr; insert { $($key:expr => $val:expr),* , }) => (
        $crate::hashmap!(with $map; insert { $($key => $val),* })
    );
    (with $map:expr; insert { $($key:expr => $val:expr),* }) => ({
        let count = $crate::const_expr_count!($($key);*);
        #[allow(unused_mut)]
        let mut map = $map;
        map.reserve(count);
        $(
            map.insert($key, $val);
        )*
        map
    });
    ($($key:expr => $val:expr),* ,) => (
        $crate::hashmap!($($key => $val),*)
    );
    ($($key:expr => $val:expr),*) => ({
        let start_capacity = $crate::const_expr_count!($($key);*);
        #[allow(unused_mut)]
        let mut map = ::std::collections::HashMap::with_capacity(start_capacity);
        $( map.insert($key, $val); )*
        map
    });
}
