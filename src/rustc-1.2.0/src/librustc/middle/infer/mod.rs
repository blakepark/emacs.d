// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! See the Book for more information.

#![allow(non_camel_case_types)]

pub use self::LateBoundRegionConversionTime::*;
pub use self::RegionVariableOrigin::*;
pub use self::SubregionOrigin::*;
pub use self::TypeOrigin::*;
pub use self::ValuePairs::*;
pub use self::fixup_err::*;
pub use middle::ty::IntVarValue;
pub use self::freshen::TypeFreshener;
pub use self::region_inference::GenericKind;

use middle::free_region::FreeRegionMap;
use middle::subst;
use middle::subst::Substs;
use middle::ty::{TyVid, IntVid, FloatVid, RegionVid, UnconstrainedNumeric};
use middle::ty::{self, Ty};
use middle::ty_fold::{self, TypeFolder, TypeFoldable};
use middle::ty_relate::{Relate, RelateResult, TypeRelation};
use rustc_data_structures::unify::{self, UnificationTable};
use std::cell::{RefCell};
use std::fmt;
use syntax::ast;
use syntax::codemap;
use syntax::codemap::Span;
use util::nodemap::FnvHashMap;

use self::combine::CombineFields;
use self::region_inference::{RegionVarBindings, RegionSnapshot};
use self::error_reporting::ErrorReporting;
use self::unify_key::ToType;

pub mod bivariate;
pub mod combine;
pub mod equate;
pub mod error_reporting;
pub mod glb;
mod higher_ranked;
pub mod lattice;
pub mod lub;
pub mod region_inference;
pub mod resolve;
mod freshen;
pub mod sub;
pub mod type_variable;
pub mod unify_key;

pub type Bound<T> = Option<T>;
pub type UnitResult<'tcx> = RelateResult<'tcx, ()>; // "unify result"
pub type fres<T> = Result<T, fixup_err>; // "fixup result"

pub struct InferCtxt<'a, 'tcx: 'a> {
    pub tcx: &'a ty::ctxt<'tcx>,

    // We instantiate UnificationTable with bounds<Ty> because the
    // types that might instantiate a general type variable have an
    // order, represented by its upper and lower bounds.
    type_variables: RefCell<type_variable::TypeVariableTable<'tcx>>,

    // Map from integral variable to the kind of integer it represents
    int_unification_table: RefCell<UnificationTable<ty::IntVid>>,

    // Map from floating variable to the kind of float it represents
    float_unification_table: RefCell<UnificationTable<ty::FloatVid>>,

    // For region variables.
    region_vars: RegionVarBindings<'a, 'tcx>,
}

/// A map returned by `skolemize_late_bound_regions()` indicating the skolemized
/// region that each late-bound region was replaced with.
pub type SkolemizationMap = FnvHashMap<ty::BoundRegion,ty::Region>;

/// Why did we require that the two types be related?
///
/// See `error_reporting.rs` for more details
#[derive(Clone, Copy, Debug)]
pub enum TypeOrigin {
    // Not yet categorized in a better way
    Misc(Span),

    // Checking that method of impl is compatible with trait
    MethodCompatCheck(Span),

    // Checking that this expression can be assigned where it needs to be
    // FIXME(eddyb) #11161 is the original Expr required?
    ExprAssignable(Span),

    // Relating trait refs when resolving vtables
    RelateTraitRefs(Span),

    // Relating self types when resolving vtables
    RelateSelfType(Span),

    // Relating trait type parameters to those found in impl etc
    RelateOutputImplTypes(Span),

    // Computing common supertype in the arms of a match expression
    MatchExpressionArm(Span, Span),

    // Computing common supertype in an if expression
    IfExpression(Span),

    // Computing common supertype of an if expression with no else counter-part
    IfExpressionWithNoElse(Span),

    // Computing common supertype in a range expression
    RangeExpression(Span),

    // `where a == b`
    EquatePredicate(Span),
}

impl TypeOrigin {
    fn as_str(&self) -> &'static str {
        match self {
            &TypeOrigin::Misc(_) |
            &TypeOrigin::RelateSelfType(_) |
            &TypeOrigin::RelateOutputImplTypes(_) |
            &TypeOrigin::ExprAssignable(_) => "mismatched types",
            &TypeOrigin::RelateTraitRefs(_) => "mismatched traits",
            &TypeOrigin::MethodCompatCheck(_) => "method not compatible with trait",
            &TypeOrigin::MatchExpressionArm(_, _) => "match arms have incompatible types",
            &TypeOrigin::IfExpression(_) => "if and else have incompatible types",
            &TypeOrigin::IfExpressionWithNoElse(_) => "if may be missing an else clause",
            &TypeOrigin::RangeExpression(_) => "start and end of range have incompatible types",
            &TypeOrigin::EquatePredicate(_) => "equality predicate not satisfied",
        }
    }
}

impl fmt::Display for TypeOrigin {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(),fmt::Error> {
        fmt::Display::fmt(self.as_str(), f)
    }
}

/// See `error_reporting.rs` for more details
#[derive(Clone, Debug)]
pub enum ValuePairs<'tcx> {
    Types(ty::expected_found<Ty<'tcx>>),
    TraitRefs(ty::expected_found<ty::TraitRef<'tcx>>),
    PolyTraitRefs(ty::expected_found<ty::PolyTraitRef<'tcx>>),
}

/// The trace designates the path through inference that we took to
/// encounter an error or subtyping constraint.
///
/// See `error_reporting.rs` for more details.
#[derive(Clone)]
pub struct TypeTrace<'tcx> {
    origin: TypeOrigin,
    values: ValuePairs<'tcx>,
}

/// The origin of a `r1 <= r2` constraint.
///
/// See `error_reporting.rs` for more details
#[derive(Clone, Debug)]
pub enum SubregionOrigin<'tcx> {
    // Arose from a subtyping relation
    Subtype(TypeTrace<'tcx>),

    // Arose from a subtyping relation
    DefaultExistentialBound(TypeTrace<'tcx>),

    // Stack-allocated closures cannot outlive innermost loop
    // or function so as to ensure we only require finite stack
    InfStackClosure(Span),

    // Invocation of closure must be within its lifetime
    InvokeClosure(Span),

    // Dereference of reference must be within its lifetime
    DerefPointer(Span),

    // Closure bound must not outlive captured free variables
    FreeVariable(Span, ast::NodeId),

    // Index into slice must be within its lifetime
    IndexSlice(Span),

    // When casting `&'a T` to an `&'b Trait` object,
    // relating `'a` to `'b`
    RelateObjectBound(Span),

    // Some type parameter was instantiated with the given type,
    // and that type must outlive some region.
    RelateParamBound(Span, Ty<'tcx>),

    // The given region parameter was instantiated with a region
    // that must outlive some other region.
    RelateRegionParamBound(Span),

    // A bound placed on type parameters that states that must outlive
    // the moment of their instantiation.
    RelateDefaultParamBound(Span, Ty<'tcx>),

    // Creating a pointer `b` to contents of another reference
    Reborrow(Span),

    // Creating a pointer `b` to contents of an upvar
    ReborrowUpvar(Span, ty::UpvarId),

    // (&'a &'b T) where a >= b
    ReferenceOutlivesReferent(Ty<'tcx>, Span),

    // The type T of an expression E must outlive the lifetime for E.
    ExprTypeIsNotInScope(Ty<'tcx>, Span),

    // A `ref b` whose region does not enclose the decl site
    BindingTypeIsNotValidAtDecl(Span),

    // Regions appearing in a method receiver must outlive method call
    CallRcvr(Span),

    // Regions appearing in a function argument must outlive func call
    CallArg(Span),

    // Region in return type of invoked fn must enclose call
    CallReturn(Span),

    // Operands must be in scope
    Operand(Span),

    // Region resulting from a `&` expr must enclose the `&` expr
    AddrOf(Span),

    // An auto-borrow that does not enclose the expr where it occurs
    AutoBorrow(Span),

    // Region constraint arriving from destructor safety
    SafeDestructor(Span),
}

/// Times when we replace late-bound regions with variables:
#[derive(Clone, Copy, Debug)]
pub enum LateBoundRegionConversionTime {
    /// when a fn is called
    FnCall,

    /// when two higher-ranked types are compared
    HigherRankedType,

    /// when projecting an associated type
    AssocTypeProjection(ast::Name),
}

/// Reasons to create a region inference variable
///
/// See `error_reporting.rs` for more details
#[derive(Clone, Debug)]
pub enum RegionVariableOrigin {
    // Region variables created for ill-categorized reasons,
    // mostly indicates places in need of refactoring
    MiscVariable(Span),

    // Regions created by a `&P` or `[...]` pattern
    PatternRegion(Span),

    // Regions created by `&` operator
    AddrOfRegion(Span),

    // Regions created as part of an autoref of a method receiver
    Autoref(Span),

    // Regions created as part of an automatic coercion
    Coercion(Span),

    // Region variables created as the values for early-bound regions
    EarlyBoundRegion(Span, ast::Name),

    // Region variables created for bound regions
    // in a function or method that is called
    LateBoundRegion(Span, ty::BoundRegion, LateBoundRegionConversionTime),

    UpvarRegion(ty::UpvarId, Span),

    BoundRegionInCoherence(ast::Name),
}

#[derive(Copy, Clone, Debug)]
pub enum fixup_err {
    unresolved_int_ty(IntVid),
    unresolved_float_ty(FloatVid),
    unresolved_ty(TyVid)
}

pub fn fixup_err_to_string(f: fixup_err) -> String {
    match f {
      unresolved_int_ty(_) => {
          "cannot determine the type of this integer; add a suffix to \
           specify the type explicitly".to_string()
      }
      unresolved_float_ty(_) => {
          "cannot determine the type of this number; add a suffix to specify \
           the type explicitly".to_string()
      }
      unresolved_ty(_) => "unconstrained type".to_string(),
    }
}

pub fn new_infer_ctxt<'a, 'tcx>(tcx: &'a ty::ctxt<'tcx>)
                                -> InferCtxt<'a, 'tcx> {
    InferCtxt {
        tcx: tcx,
        type_variables: RefCell::new(type_variable::TypeVariableTable::new()),
        int_unification_table: RefCell::new(UnificationTable::new()),
        float_unification_table: RefCell::new(UnificationTable::new()),
        region_vars: RegionVarBindings::new(tcx),
    }
}

/// Computes the least upper-bound of `a` and `b`. If this is not possible, reports an error and
/// returns ty::err.
pub fn common_supertype<'a, 'tcx>(cx: &InferCtxt<'a, 'tcx>,
                                  origin: TypeOrigin,
                                  a_is_expected: bool,
                                  a: Ty<'tcx>,
                                  b: Ty<'tcx>)
                                  -> Ty<'tcx>
{
    debug!("common_supertype({:?}, {:?})",
           a, b);

    let trace = TypeTrace {
        origin: origin,
        values: Types(expected_found(a_is_expected, a, b))
    };

    let result = cx.commit_if_ok(|_| cx.lub(a_is_expected, trace.clone()).relate(&a, &b));
    match result {
        Ok(t) => t,
        Err(ref err) => {
            cx.report_and_explain_type_error(trace, err);
            cx.tcx.types.err
        }
    }
}

pub fn mk_subty<'a, 'tcx>(cx: &InferCtxt<'a, 'tcx>,
                          a_is_expected: bool,
                          origin: TypeOrigin,
                          a: Ty<'tcx>,
                          b: Ty<'tcx>)
                          -> UnitResult<'tcx>
{
    debug!("mk_subty({:?} <: {:?})", a, b);
    cx.sub_types(a_is_expected, origin, a, b)
}

pub fn can_mk_subty<'a, 'tcx>(cx: &InferCtxt<'a, 'tcx>,
                              a: Ty<'tcx>,
                              b: Ty<'tcx>)
                              -> UnitResult<'tcx> {
    debug!("can_mk_subty({:?} <: {:?})", a, b);
    cx.probe(|_| {
        let trace = TypeTrace {
            origin: Misc(codemap::DUMMY_SP),
            values: Types(expected_found(true, a, b))
        };
        cx.sub(true, trace).relate(&a, &b).map(|_| ())
    })
}

pub fn can_mk_eqty<'a, 'tcx>(cx: &InferCtxt<'a, 'tcx>, a: Ty<'tcx>, b: Ty<'tcx>)
                             -> UnitResult<'tcx>
{
    cx.can_equate(&a, &b)
}

pub fn mk_subr<'a, 'tcx>(cx: &InferCtxt<'a, 'tcx>,
                         origin: SubregionOrigin<'tcx>,
                         a: ty::Region,
                         b: ty::Region) {
    debug!("mk_subr({:?} <: {:?})", a, b);
    let snapshot = cx.region_vars.start_snapshot();
    cx.region_vars.make_subregion(origin, a, b);
    cx.region_vars.commit(snapshot);
}

pub fn mk_eqty<'a, 'tcx>(cx: &InferCtxt<'a, 'tcx>,
                         a_is_expected: bool,
                         origin: TypeOrigin,
                         a: Ty<'tcx>,
                         b: Ty<'tcx>)
                         -> UnitResult<'tcx>
{
    debug!("mk_eqty({:?} <: {:?})", a, b);
    cx.commit_if_ok(|_| cx.eq_types(a_is_expected, origin, a, b))
}

pub fn mk_sub_poly_trait_refs<'a, 'tcx>(cx: &InferCtxt<'a, 'tcx>,
                                   a_is_expected: bool,
                                   origin: TypeOrigin,
                                   a: ty::PolyTraitRef<'tcx>,
                                   b: ty::PolyTraitRef<'tcx>)
                                   -> UnitResult<'tcx>
{
    debug!("mk_sub_trait_refs({:?} <: {:?})",
           a, b);
    cx.commit_if_ok(|_| cx.sub_poly_trait_refs(a_is_expected, origin, a.clone(), b.clone()))
}

fn expected_found<T>(a_is_expected: bool,
                     a: T,
                     b: T)
                     -> ty::expected_found<T>
{
    if a_is_expected {
        ty::expected_found {expected: a, found: b}
    } else {
        ty::expected_found {expected: b, found: a}
    }
}

#[must_use = "once you start a snapshot, you should always consume it"]
pub struct CombinedSnapshot {
    type_snapshot: type_variable::Snapshot,
    int_snapshot: unify::Snapshot<ty::IntVid>,
    float_snapshot: unify::Snapshot<ty::FloatVid>,
    region_vars_snapshot: RegionSnapshot,
}

impl<'a, 'tcx> InferCtxt<'a, 'tcx> {
    pub fn freshen<T:TypeFoldable<'tcx>>(&self, t: T) -> T {
        t.fold_with(&mut self.freshener())
    }

    pub fn type_var_diverges(&'a self, ty: Ty) -> bool {
        match ty.sty {
            ty::TyInfer(ty::TyVar(vid)) => self.type_variables.borrow().var_diverges(vid),
            _ => false
        }
    }

    pub fn freshener<'b>(&'b self) -> TypeFreshener<'b, 'tcx> {
        freshen::TypeFreshener::new(self)
    }

    pub fn type_is_unconstrained_numeric(&'a self, ty: Ty) -> UnconstrainedNumeric {
        use middle::ty::UnconstrainedNumeric::{Neither, UnconstrainedInt, UnconstrainedFloat};
        match ty.sty {
            ty::TyInfer(ty::IntVar(vid)) => {
                if self.int_unification_table.borrow_mut().has_value(vid) {
                    Neither
                } else {
                    UnconstrainedInt
                }
            },
            ty::TyInfer(ty::FloatVar(vid)) => {
                if self.float_unification_table.borrow_mut().has_value(vid) {
                    Neither
                } else {
                    UnconstrainedFloat
                }
            },
            _ => Neither,
        }
    }

    fn combine_fields(&'a self, a_is_expected: bool, trace: TypeTrace<'tcx>)
                      -> CombineFields<'a, 'tcx> {
        CombineFields {infcx: self,
                       a_is_expected: a_is_expected,
                       trace: trace,
                       cause: None}
    }

    // public so that it can be used from the rustc_driver unit tests
    pub fn equate(&'a self, a_is_expected: bool, trace: TypeTrace<'tcx>)
              -> equate::Equate<'a, 'tcx>
    {
        self.combine_fields(a_is_expected, trace).equate()
    }

    // public so that it can be used from the rustc_driver unit tests
    pub fn sub(&'a self, a_is_expected: bool, trace: TypeTrace<'tcx>)
               -> sub::Sub<'a, 'tcx>
    {
        self.combine_fields(a_is_expected, trace).sub()
    }

    // public so that it can be used from the rustc_driver unit tests
    pub fn lub(&'a self, a_is_expected: bool, trace: TypeTrace<'tcx>)
               -> lub::Lub<'a, 'tcx>
    {
        self.combine_fields(a_is_expected, trace).lub()
    }

    // public so that it can be used from the rustc_driver unit tests
    pub fn glb(&'a self, a_is_expected: bool, trace: TypeTrace<'tcx>)
               -> glb::Glb<'a, 'tcx>
    {
        self.combine_fields(a_is_expected, trace).glb()
    }

    fn start_snapshot(&self) -> CombinedSnapshot {
        CombinedSnapshot {
            type_snapshot: self.type_variables.borrow_mut().snapshot(),
            int_snapshot: self.int_unification_table.borrow_mut().snapshot(),
            float_snapshot: self.float_unification_table.borrow_mut().snapshot(),
            region_vars_snapshot: self.region_vars.start_snapshot(),
        }
    }

    fn rollback_to(&self, snapshot: CombinedSnapshot) {
        debug!("rollback!");
        let CombinedSnapshot { type_snapshot,
                               int_snapshot,
                               float_snapshot,
                               region_vars_snapshot } = snapshot;

        self.type_variables
            .borrow_mut()
            .rollback_to(type_snapshot);
        self.int_unification_table
            .borrow_mut()
            .rollback_to(int_snapshot);
        self.float_unification_table
            .borrow_mut()
            .rollback_to(float_snapshot);
        self.region_vars
            .rollback_to(region_vars_snapshot);
    }

    fn commit_from(&self, snapshot: CombinedSnapshot) {
        debug!("commit_from!");
        let CombinedSnapshot { type_snapshot,
                               int_snapshot,
                               float_snapshot,
                               region_vars_snapshot } = snapshot;

        self.type_variables
            .borrow_mut()
            .commit(type_snapshot);
        self.int_unification_table
            .borrow_mut()
            .commit(int_snapshot);
        self.float_unification_table
            .borrow_mut()
            .commit(float_snapshot);
        self.region_vars
            .commit(region_vars_snapshot);
    }

    /// Execute `f` and commit the bindings
    pub fn commit_unconditionally<R, F>(&self, f: F) -> R where
        F: FnOnce() -> R,
    {
        debug!("commit()");
        let snapshot = self.start_snapshot();
        let r = f();
        self.commit_from(snapshot);
        r
    }

    /// Execute `f` and commit the bindings if closure `f` returns `Ok(_)`
    pub fn commit_if_ok<T, E, F>(&self, f: F) -> Result<T, E> where
        F: FnOnce(&CombinedSnapshot) -> Result<T, E>
    {
        debug!("commit_if_ok()");
        let snapshot = self.start_snapshot();
        let r = f(&snapshot);
        debug!("commit_if_ok() -- r.is_ok() = {}", r.is_ok());
        match r {
            Ok(_) => { self.commit_from(snapshot); }
            Err(_) => { self.rollback_to(snapshot); }
        }
        r
    }

    /// Execute `f` and commit only the region bindings if successful.
    /// The function f must be very careful not to leak any non-region
    /// variables that get created.
    pub fn commit_regions_if_ok<T, E, F>(&self, f: F) -> Result<T, E> where
        F: FnOnce() -> Result<T, E>
    {
        debug!("commit_regions_if_ok()");
        let CombinedSnapshot { type_snapshot,
                               int_snapshot,
                               float_snapshot,
                               region_vars_snapshot } = self.start_snapshot();

        let r = self.commit_if_ok(|_| f());

        // Roll back any non-region bindings - they should be resolved
        // inside `f`, with, e.g. `resolve_type_vars_if_possible`.
        self.type_variables
            .borrow_mut()
            .rollback_to(type_snapshot);
        self.int_unification_table
            .borrow_mut()
            .rollback_to(int_snapshot);
        self.float_unification_table
            .borrow_mut()
            .rollback_to(float_snapshot);

        // Commit region vars that may escape through resolved types.
        self.region_vars
            .commit(region_vars_snapshot);

        r
    }

    /// Execute `f` then unroll any bindings it creates
    pub fn probe<R, F>(&self, f: F) -> R where
        F: FnOnce(&CombinedSnapshot) -> R,
    {
        debug!("probe()");
        let snapshot = self.start_snapshot();
        let r = f(&snapshot);
        self.rollback_to(snapshot);
        r
    }

    pub fn add_given(&self,
                     sub: ty::FreeRegion,
                     sup: ty::RegionVid)
    {
        self.region_vars.add_given(sub, sup);
    }

    pub fn sub_types(&self,
                     a_is_expected: bool,
                     origin: TypeOrigin,
                     a: Ty<'tcx>,
                     b: Ty<'tcx>)
                     -> UnitResult<'tcx>
    {
        debug!("sub_types({:?} <: {:?})", a, b);
        self.commit_if_ok(|_| {
            let trace = TypeTrace::types(origin, a_is_expected, a, b);
            self.sub(a_is_expected, trace).relate(&a, &b).map(|_| ())
        })
    }

    pub fn eq_types(&self,
                    a_is_expected: bool,
                    origin: TypeOrigin,
                    a: Ty<'tcx>,
                    b: Ty<'tcx>)
                    -> UnitResult<'tcx>
    {
        self.commit_if_ok(|_| {
            let trace = TypeTrace::types(origin, a_is_expected, a, b);
            self.equate(a_is_expected, trace).relate(&a, &b).map(|_| ())
        })
    }

    pub fn sub_trait_refs(&self,
                          a_is_expected: bool,
                          origin: TypeOrigin,
                          a: ty::TraitRef<'tcx>,
                          b: ty::TraitRef<'tcx>)
                          -> UnitResult<'tcx>
    {
        debug!("sub_trait_refs({:?} <: {:?})",
               a,
               b);
        self.commit_if_ok(|_| {
            let trace = TypeTrace {
                origin: origin,
                values: TraitRefs(expected_found(a_is_expected, a.clone(), b.clone()))
            };
            self.sub(a_is_expected, trace).relate(&a, &b).map(|_| ())
        })
    }

    pub fn sub_poly_trait_refs(&self,
                               a_is_expected: bool,
                               origin: TypeOrigin,
                               a: ty::PolyTraitRef<'tcx>,
                               b: ty::PolyTraitRef<'tcx>)
                               -> UnitResult<'tcx>
    {
        debug!("sub_poly_trait_refs({:?} <: {:?})",
               a,
               b);
        self.commit_if_ok(|_| {
            let trace = TypeTrace {
                origin: origin,
                values: PolyTraitRefs(expected_found(a_is_expected, a.clone(), b.clone()))
            };
            self.sub(a_is_expected, trace).relate(&a, &b).map(|_| ())
        })
    }

    pub fn construct_skolemized_subst(&self,
                                      generics: &ty::Generics<'tcx>,
                                      snapshot: &CombinedSnapshot)
                                      -> (subst::Substs<'tcx>, SkolemizationMap) {
        /*! See `higher_ranked::construct_skolemized_subst` */

        higher_ranked::construct_skolemized_substs(self, generics, snapshot)
    }

    pub fn skolemize_late_bound_regions<T>(&self,
                                           value: &ty::Binder<T>,
                                           snapshot: &CombinedSnapshot)
                                           -> (T, SkolemizationMap)
        where T : TypeFoldable<'tcx>
    {
        /*! See `higher_ranked::skolemize_late_bound_regions` */

        higher_ranked::skolemize_late_bound_regions(self, value, snapshot)
    }

    pub fn leak_check(&self,
                      skol_map: &SkolemizationMap,
                      snapshot: &CombinedSnapshot)
                      -> UnitResult<'tcx>
    {
        /*! See `higher_ranked::leak_check` */

        match higher_ranked::leak_check(self, skol_map, snapshot) {
            Ok(()) => Ok(()),
            Err((br, r)) => Err(ty::terr_regions_insufficiently_polymorphic(br, r))
        }
    }

    pub fn plug_leaks<T>(&self,
                         skol_map: SkolemizationMap,
                         snapshot: &CombinedSnapshot,
                         value: &T)
                         -> T
        where T : TypeFoldable<'tcx>
    {
        /*! See `higher_ranked::plug_leaks` */

        higher_ranked::plug_leaks(self, skol_map, snapshot, value)
    }

    pub fn equality_predicate(&self,
                              span: Span,
                              predicate: &ty::PolyEquatePredicate<'tcx>)
                              -> UnitResult<'tcx> {
        self.commit_if_ok(|snapshot| {
            let (ty::EquatePredicate(a, b), skol_map) =
                self.skolemize_late_bound_regions(predicate, snapshot);
            let origin = EquatePredicate(span);
            let () = try!(mk_eqty(self, false, origin, a, b));
            self.leak_check(&skol_map, snapshot)
        })
    }

    pub fn region_outlives_predicate(&self,
                                     span: Span,
                                     predicate: &ty::PolyRegionOutlivesPredicate)
                                     -> UnitResult<'tcx> {
        self.commit_if_ok(|snapshot| {
            let (ty::OutlivesPredicate(r_a, r_b), skol_map) =
                self.skolemize_late_bound_regions(predicate, snapshot);
            let origin = RelateRegionParamBound(span);
            let () = mk_subr(self, origin, r_b, r_a); // `b : a` ==> `a <= b`
            self.leak_check(&skol_map, snapshot)
        })
    }

    pub fn next_ty_var_id(&self, diverging: bool) -> TyVid {
        self.type_variables
            .borrow_mut()
            .new_var(diverging)
    }

    pub fn next_ty_var(&self) -> Ty<'tcx> {
        ty::mk_var(self.tcx, self.next_ty_var_id(false))
    }

    pub fn next_diverging_ty_var(&self) -> Ty<'tcx> {
        ty::mk_var(self.tcx, self.next_ty_var_id(true))
    }

    pub fn next_ty_vars(&self, n: usize) -> Vec<Ty<'tcx>> {
        (0..n).map(|_i| self.next_ty_var()).collect()
    }

    pub fn next_int_var_id(&self) -> IntVid {
        self.int_unification_table
            .borrow_mut()
            .new_key(None)
    }

    pub fn next_float_var_id(&self) -> FloatVid {
        self.float_unification_table
            .borrow_mut()
            .new_key(None)
    }

    pub fn next_region_var(&self, origin: RegionVariableOrigin) -> ty::Region {
        ty::ReInfer(ty::ReVar(self.region_vars.new_region_var(origin)))
    }

    pub fn region_vars_for_defs(&self,
                                span: Span,
                                defs: &[ty::RegionParameterDef])
                                -> Vec<ty::Region> {
        defs.iter()
            .map(|d| self.next_region_var(EarlyBoundRegion(span, d.name)))
            .collect()
    }

    /// Given a set of generics defined on a type or impl, returns a substitution mapping each
    /// type/region parameter to a fresh inference variable.
    pub fn fresh_substs_for_generics(&self,
                                     span: Span,
                                     generics: &ty::Generics<'tcx>)
                                     -> subst::Substs<'tcx>
    {
        let type_params =
            generics.types.map(
                |_| self.next_ty_var());
        let region_params =
            generics.regions.map(
                |d| self.next_region_var(EarlyBoundRegion(span, d.name)));
        subst::Substs::new(type_params, region_params)
    }

    /// Given a set of generics defined on a trait, returns a substitution mapping each output
    /// type/region parameter to a fresh inference variable, and mapping the self type to
    /// `self_ty`.
    pub fn fresh_substs_for_trait(&self,
                                  span: Span,
                                  generics: &ty::Generics<'tcx>,
                                  self_ty: Ty<'tcx>)
                                  -> subst::Substs<'tcx>
    {

        assert!(generics.types.len(subst::SelfSpace) == 1);
        assert!(generics.types.len(subst::FnSpace) == 0);
        assert!(generics.regions.len(subst::SelfSpace) == 0);
        assert!(generics.regions.len(subst::FnSpace) == 0);

        let type_parameter_count = generics.types.len(subst::TypeSpace);
        let type_parameters = self.next_ty_vars(type_parameter_count);

        let region_param_defs = generics.regions.get_slice(subst::TypeSpace);
        let regions = self.region_vars_for_defs(span, region_param_defs);

        subst::Substs::new_trait(type_parameters, regions, self_ty)
    }

    pub fn fresh_bound_region(&self, debruijn: ty::DebruijnIndex) -> ty::Region {
        self.region_vars.new_bound(debruijn)
    }

    pub fn resolve_regions_and_report_errors(&self,
                                             free_regions: &FreeRegionMap,
                                             subject_node_id: ast::NodeId) {
        let errors = self.region_vars.resolve_regions(free_regions, subject_node_id);
        self.report_region_errors(&errors); // see error_reporting.rs
    }

    pub fn ty_to_string(&self, t: Ty<'tcx>) -> String {
        self.resolve_type_vars_if_possible(&t).to_string()
    }

    pub fn tys_to_string(&self, ts: &[Ty<'tcx>]) -> String {
        let tstrs: Vec<String> = ts.iter().map(|t| self.ty_to_string(*t)).collect();
        format!("({})", tstrs.connect(", "))
    }

    pub fn trait_ref_to_string(&self, t: &ty::TraitRef<'tcx>) -> String {
        self.resolve_type_vars_if_possible(t).to_string()
    }

    pub fn shallow_resolve(&self, typ: Ty<'tcx>) -> Ty<'tcx> {
        match typ.sty {
            ty::TyInfer(ty::TyVar(v)) => {
                // Not entirely obvious: if `typ` is a type variable,
                // it can be resolved to an int/float variable, which
                // can then be recursively resolved, hence the
                // recursion. Note though that we prevent type
                // variables from unifying to other type variables
                // directly (though they may be embedded
                // structurally), and we prevent cycles in any case,
                // so this recursion should always be of very limited
                // depth.
                self.type_variables.borrow()
                    .probe(v)
                    .map(|t| self.shallow_resolve(t))
                    .unwrap_or(typ)
            }

            ty::TyInfer(ty::IntVar(v)) => {
                self.int_unification_table
                    .borrow_mut()
                    .probe(v)
                    .map(|v| v.to_type(self.tcx))
                    .unwrap_or(typ)
            }

            ty::TyInfer(ty::FloatVar(v)) => {
                self.float_unification_table
                    .borrow_mut()
                    .probe(v)
                    .map(|v| v.to_type(self.tcx))
                    .unwrap_or(typ)
            }

            _ => {
                typ
            }
        }
    }

    pub fn resolve_type_vars_if_possible<T:TypeFoldable<'tcx>>(&self, value: &T) -> T {
        /*!
         * Where possible, replaces type/int/float variables in
         * `value` with their final value. Note that region variables
         * are unaffected. If a type variable has not been unified, it
         * is left as is.  This is an idempotent operation that does
         * not affect inference state in any way and so you can do it
         * at will.
         */

        let mut r = resolve::OpportunisticTypeResolver::new(self);
        value.fold_with(&mut r)
    }

    pub fn fully_resolve<T:TypeFoldable<'tcx>>(&self, value: &T) -> fres<T> {
        /*!
         * Attempts to resolve all type/region variables in
         * `value`. Region inference must have been run already (e.g.,
         * by calling `resolve_regions_and_report_errors`).  If some
         * variable was never unified, an `Err` results.
         *
         * This method is idempotent, but it not typically not invoked
         * except during the writeback phase.
         */

        resolve::fully_resolve(self, value)
    }

    // [Note-Type-error-reporting]
    // An invariant is that anytime the expected or actual type is TyError (the special
    // error type, meaning that an error occurred when typechecking this expression),
    // this is a derived error. The error cascaded from another error (that was already
    // reported), so it's not useful to display it to the user.
    // The following four methods -- type_error_message_str, type_error_message_str_with_expected,
    // type_error_message, and report_mismatched_types -- implement this logic.
    // They check if either the actual or expected type is TyError, and don't print the error
    // in this case. The typechecker should only ever report type errors involving mismatched
    // types using one of these four methods, and should not call span_err directly for such
    // errors.
    pub fn type_error_message_str<M>(&self,
                                     sp: Span,
                                     mk_msg: M,
                                     actual_ty: String,
                                     err: Option<&ty::type_err<'tcx>>) where
        M: FnOnce(Option<String>, String) -> String,
    {
        self.type_error_message_str_with_expected(sp, mk_msg, None, actual_ty, err)
    }

    pub fn type_error_message_str_with_expected<M>(&self,
                                                   sp: Span,
                                                   mk_msg: M,
                                                   expected_ty: Option<Ty<'tcx>>,
                                                   actual_ty: String,
                                                   err: Option<&ty::type_err<'tcx>>) where
        M: FnOnce(Option<String>, String) -> String,
    {
        debug!("hi! expected_ty = {:?}, actual_ty = {}", expected_ty, actual_ty);

        let resolved_expected = expected_ty.map(|e_ty| self.resolve_type_vars_if_possible(&e_ty));

        match resolved_expected {
            Some(t) if ty::type_is_error(t) => (),
            _ => {
                let error_str = err.map_or("".to_string(), |t_err| {
                    format!(" ({})", t_err)
                });

                self.tcx.sess.span_err(sp, &format!("{}{}",
                    mk_msg(resolved_expected.map(|t| self.ty_to_string(t)), actual_ty),
                    error_str));

                if let Some(err) = err {
                    ty::note_and_explain_type_err(self.tcx, err, sp)
                }
            }
        }
    }

    pub fn type_error_message<M>(&self,
                                 sp: Span,
                                 mk_msg: M,
                                 actual_ty: Ty<'tcx>,
                                 err: Option<&ty::type_err<'tcx>>) where
        M: FnOnce(String) -> String,
    {
        let actual_ty = self.resolve_type_vars_if_possible(&actual_ty);

        // Don't report an error if actual type is TyError.
        if ty::type_is_error(actual_ty) {
            return;
        }

        self.type_error_message_str(sp,
            move |_e, a| { mk_msg(a) },
            self.ty_to_string(actual_ty), err);
    }

    pub fn report_mismatched_types(&self,
                                   span: Span,
                                   expected: Ty<'tcx>,
                                   actual: Ty<'tcx>,
                                   err: &ty::type_err<'tcx>) {
        let trace = TypeTrace {
            origin: Misc(span),
            values: Types(ty::expected_found {
                expected: expected,
                found: actual
            })
        };
        self.report_and_explain_type_error(trace, err);
    }

    pub fn replace_late_bound_regions_with_fresh_var<T>(
        &self,
        span: Span,
        lbrct: LateBoundRegionConversionTime,
        value: &ty::Binder<T>)
        -> (T, FnvHashMap<ty::BoundRegion,ty::Region>)
        where T : TypeFoldable<'tcx>
    {
        ty_fold::replace_late_bound_regions(
            self.tcx,
            value,
            |br| self.next_region_var(LateBoundRegion(span, br, lbrct)))
    }

    /// See `verify_generic_bound` method in `region_inference`
    pub fn verify_generic_bound(&self,
                                origin: SubregionOrigin<'tcx>,
                                kind: GenericKind<'tcx>,
                                a: ty::Region,
                                bs: Vec<ty::Region>) {
        debug!("verify_generic_bound({:?}, {:?} <: {:?})",
               kind,
               a,
               bs);

        self.region_vars.verify_generic_bound(origin, kind, a, bs);
    }

    pub fn can_equate<'b,T>(&'b self, a: &T, b: &T) -> UnitResult<'tcx>
        where T: Relate<'b,'tcx> + fmt::Debug
    {
        debug!("can_equate({:?}, {:?})", a, b);
        self.probe(|_| {
            // Gin up a dummy trace, since this won't be committed
            // anyhow. We should make this typetrace stuff more
            // generic so we don't have to do anything quite this
            // terrible.
            let e = self.tcx.types.err;
            let trace = TypeTrace { origin: Misc(codemap::DUMMY_SP),
                                    values: Types(expected_found(true, e, e)) };
            self.equate(true, trace).relate(a, b)
        }).map(|_| ())
    }
}

impl<'tcx> TypeTrace<'tcx> {
    pub fn span(&self) -> Span {
        self.origin.span()
    }

    pub fn types(origin: TypeOrigin,
                 a_is_expected: bool,
                 a: Ty<'tcx>,
                 b: Ty<'tcx>)
                 -> TypeTrace<'tcx> {
        TypeTrace {
            origin: origin,
            values: Types(expected_found(a_is_expected, a, b))
        }
    }

    pub fn dummy(tcx: &ty::ctxt<'tcx>) -> TypeTrace<'tcx> {
        TypeTrace {
            origin: Misc(codemap::DUMMY_SP),
            values: Types(ty::expected_found {
                expected: tcx.types.err,
                found: tcx.types.err,
            })
        }
    }
}

impl<'tcx> fmt::Debug for TypeTrace<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TypeTrace({:?})", self.origin)
    }
}

impl TypeOrigin {
    pub fn span(&self) -> Span {
        match *self {
            MethodCompatCheck(span) => span,
            ExprAssignable(span) => span,
            Misc(span) => span,
            RelateTraitRefs(span) => span,
            RelateSelfType(span) => span,
            RelateOutputImplTypes(span) => span,
            MatchExpressionArm(match_span, _) => match_span,
            IfExpression(span) => span,
            IfExpressionWithNoElse(span) => span,
            RangeExpression(span) => span,
            EquatePredicate(span) => span,
        }
    }
}

impl<'tcx> SubregionOrigin<'tcx> {
    pub fn span(&self) -> Span {
        match *self {
            Subtype(ref a) => a.span(),
            DefaultExistentialBound(ref a) => a.span(),
            InfStackClosure(a) => a,
            InvokeClosure(a) => a,
            DerefPointer(a) => a,
            FreeVariable(a, _) => a,
            IndexSlice(a) => a,
            RelateObjectBound(a) => a,
            RelateParamBound(a, _) => a,
            RelateRegionParamBound(a) => a,
            RelateDefaultParamBound(a, _) => a,
            Reborrow(a) => a,
            ReborrowUpvar(a, _) => a,
            ReferenceOutlivesReferent(_, a) => a,
            ExprTypeIsNotInScope(_, a) => a,
            BindingTypeIsNotValidAtDecl(a) => a,
            CallRcvr(a) => a,
            CallArg(a) => a,
            CallReturn(a) => a,
            Operand(a) => a,
            AddrOf(a) => a,
            AutoBorrow(a) => a,
            SafeDestructor(a) => a,
        }
    }
}

impl RegionVariableOrigin {
    pub fn span(&self) -> Span {
        match *self {
            MiscVariable(a) => a,
            PatternRegion(a) => a,
            AddrOfRegion(a) => a,
            Autoref(a) => a,
            Coercion(a) => a,
            EarlyBoundRegion(a, _) => a,
            LateBoundRegion(a, _, _) => a,
            BoundRegionInCoherence(_) => codemap::DUMMY_SP,
            UpvarRegion(_, a) => a
        }
    }
}
