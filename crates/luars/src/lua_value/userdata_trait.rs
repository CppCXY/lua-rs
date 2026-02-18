pub trait UserDataTrait: 'static {
    fn type_name(&self) -> &'static str;
}