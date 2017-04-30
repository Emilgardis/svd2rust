use svd;
error_chain! {
    links {
        SvdError(svd::errors::Error, svd::errors::ErrorKind);
    }
}
