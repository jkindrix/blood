(** * Blood — Fiber Safety via Tier-Based Crossing Rules

    Formalizes the safety of Blood's tier-based concurrency model.
    Blood uses memory tiers (Stack, Region, Persistent) with crossing
    rules to prevent data races without Rust-style Send/Sync traits.

    Reference: CONCURRENCY.md §8, MEMORY_MODEL.md §7.8
    Phase: Phase 10 — Tier 3 (Tier-Based Concurrency Safety)

    Key insight (CONCURRENCY.md §9.2): Data race freedom follows
    by construction from the tier crossing rules:
    - Stack references are fiber-local (cannot cross)
    - Mutable region references are fiber-local (cannot cross)
    - Frozen region references can cross (deeply immutable)
    - Persistent references can cross (reference-counted)

    The type system enforces these rules at compile time.
    Generation checks provide a runtime safety net for region references.

    Status: 0 Admitted.
*)

From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import PeanoNat.
From Stdlib Require Import Lia.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Semantics.
From Blood Require Import GenerationSnapshots.
From Blood Require Import Regions.

(** ** Memory tiers

    Reference: MEMORY_MODEL.md §3.1

    Tier 0 = Stack:      Lexical scope, zero-cost, fiber-local
    Tier 1 = Region:     Explicit scope, generation-checked
    Tier 2/3 = Persistent: Reference-counted, freely shareable *)

Inductive memory_tier : Type :=
  | Tier_Stack
  | Tier_Region
  | Tier_Persistent.

(** ** Mutability annotation

    Frozen references are deeply immutable and safe to share
    across fiber boundaries. Mutable references are fiber-local. *)

Inductive mutability : Type :=
  | Mut_Mutable
  | Mut_Frozen.

(** ** Typed reference with tier and mutability annotations

    Models a reference (pointer) annotated with its memory tier
    and mutability. This is the unit of fiber-crossing analysis. *)

Record typed_ref := mk_typed_ref {
  ref_tier : memory_tier;
  ref_mut : mutability;
  ref_addr : nat;
  ref_gen : nat;
}.

(** ** Fiber identifiers *)

Definition fiber_id := nat.

(** ** Fiber-crossing predicate

    Reference: CONCURRENCY.md §8.1

    Determines whether a typed reference can cross fiber boundaries.
    This is enforced at compile time by the type system.

    | Tier       | Mutable | Frozen |
    |------------|---------|--------|
    | Stack      | No      | No     |
    | Region     | No      | Yes    |
    | Persistent | Yes     | Yes    | *)

Definition can_cross_fiber (r : typed_ref) : Prop :=
  match ref_tier r with
  | Tier_Stack => False
  | Tier_Region =>
      match ref_mut r with
      | Mut_Mutable => False
      | Mut_Frozen => True
      end
  | Tier_Persistent => True
  end.

(** Boolean decision procedure *)

Definition can_cross_fiber_dec (r : typed_ref) : bool :=
  match ref_tier r with
  | Tier_Stack => false
  | Tier_Region =>
      match ref_mut r with
      | Mut_Mutable => false
      | Mut_Frozen => true
      end
  | Tier_Persistent => true
  end.

Lemma can_cross_fiber_dec_correct :
  forall r,
    can_cross_fiber_dec r = true <-> can_cross_fiber r.
Proof.
  intros r. unfold can_cross_fiber_dec, can_cross_fiber.
  destruct (ref_tier r).
  - split; [discriminate | contradiction].
  - destruct (ref_mut r); split; [discriminate | contradiction | auto | auto].
  - split; auto.
Qed.

(** ** Writability predicate *)

Definition is_writable (r : typed_ref) : Prop :=
  ref_mut r = Mut_Mutable.

(** ** Theorem 1: Stack references cannot cross fiber boundaries

    Reference: CONCURRENCY.md §8.1 — Tier 0 (stack)

    Stack memory is lexically scoped to the creating fiber's stack frame.
    References into stack memory cannot be transferred or shared. *)

Theorem stack_no_cross :
  forall r,
    ref_tier r = Tier_Stack ->
    ~ can_cross_fiber r.
Proof.
  intros r Htier Hcross.
  unfold can_cross_fiber in Hcross.
  rewrite Htier in Hcross.
  exact Hcross.
Qed.

(** Corollary: mutable region references cannot cross *)

Lemma mutable_region_no_cross :
  forall r,
    ref_tier r = Tier_Region ->
    ref_mut r = Mut_Mutable ->
    ~ can_cross_fiber r.
Proof.
  intros r Htier Hmut Hcross.
  unfold can_cross_fiber in Hcross.
  rewrite Htier in Hcross. rewrite Hmut in Hcross.
  exact Hcross.
Qed.

(** ** Theorem 3: Persistent references cross freely

    Reference: CONCURRENCY.md §8.1 — Tier 2/3 (persistent)

    Persistent-tier references are reference-counted and can be freely
    transferred between fibers regardless of mutability. *)

Theorem persistent_free_cross :
  forall r,
    ref_tier r = Tier_Persistent ->
    can_cross_fiber r.
Proof.
  intros r Htier.
  unfold can_cross_fiber. rewrite Htier. exact I.
Qed.

(** Frozen region references can cross *)

Lemma frozen_region_can_cross :
  forall r,
    ref_tier r = Tier_Region ->
    ref_mut r = Mut_Frozen ->
    can_cross_fiber r.
Proof.
  intros r Htier Hmut.
  unfold can_cross_fiber. rewrite Htier. rewrite Hmut. exact I.
Qed.

(** ** Key lemma: crossing region references must be frozen

    If a region reference can cross fiber boundaries, it must be frozen.
    This is the core property that prevents mutable aliasing for regions. *)

Lemma crossing_region_is_frozen :
  forall r,
    ref_tier r = Tier_Region ->
    can_cross_fiber r ->
    ref_mut r = Mut_Frozen.
Proof.
  intros r Htier Hcross.
  unfold can_cross_fiber in Hcross. rewrite Htier in Hcross.
  destruct (ref_mut r) eqn:Hmut.
  - contradiction.
  - reflexivity.
Qed.

(** Crossing references from regions are not writable *)

Lemma crossing_region_not_writable :
  forall r,
    ref_tier r = Tier_Region ->
    can_cross_fiber r ->
    ~ is_writable r.
Proof.
  intros r Htier Hcross Hwrite.
  unfold is_writable in Hwrite.
  assert (Hfrozen := crossing_region_is_frozen r Htier Hcross).
  rewrite Hwrite in Hfrozen. discriminate.
Qed.

(** ** Theorem 2: Region crossing requires generation check

    Reference: MEMORY_MODEL.md §7.8, Regions.v

    If a frozen reference to a region-allocated address crosses to
    another fiber, and the owning fiber subsequently destroys the
    region, the generation check detects the staleness.

    This theorem connects the tier crossing rules with the region
    safety infrastructure from Phase 5 (Regions.v). *)

Theorem region_checked_cross :
  forall r rgn M,
    ref_tier r = Tier_Region ->
    ref_mut r = Mut_Frozen ->
    can_cross_fiber r ->
    NoDup rgn ->
    In (ref_addr r) rgn ->
    ref_gen r = current_gen M (ref_addr r) ->
    (** After region destruction, generation mismatch is detected *)
    current_gen (region_destroy rgn M) (ref_addr r) <> ref_gen r.
Proof.
  intros r rgn M _Htier _Hmut _Hcross Hnd Hin Hgen.
  rewrite Hgen.
  rewrite (region_destroy_bumps_gen rgn M (ref_addr r) Hin Hnd).
  lia.
Qed.

(** ** Ownership model for tier-crossing safety

    To prove data race freedom, we parameterize over an address
    ownership function. This models the runtime invariant that every
    memory address has a unique owning fiber.

    When the Section closes, addr_owner becomes a function parameter. *)

Section FiberSafety.

Variable addr_owner : nat -> fiber_id.

(** ** Legally held reference

    A reference is legally held by a fiber if:
    - Mutable references: the fiber owns the address
    - Frozen references: any fiber can hold them (read-only) *)

Definition legally_held (r : typed_ref) (fid : fiber_id) : Prop :=
  match ref_mut r with
  | Mut_Mutable => addr_owner (ref_addr r) = fid
  | Mut_Frozen => True
  end.

(** Writable references require ownership *)

Lemma legally_held_writable_is_owner :
  forall r fid,
    legally_held r fid ->
    is_writable r ->
    addr_owner (ref_addr r) = fid.
Proof.
  intros r fid Hheld Hwrite.
  unfold is_writable in Hwrite. unfold legally_held in Hheld.
  rewrite Hwrite in Hheld. exact Hheld.
Qed.

(** ** Theorem 4: Tier crossing safety (data race freedom)

    Reference: CONCURRENCY.md §9.2

    If two different fibers both legally hold references to the same
    address, at most one reference can be writable. This prevents
    data races by construction.

    Proof: Both writable implies addr_owner(addr) = f1 and
    addr_owner(addr) = f2. Since addr_owner is a function, f1 = f2.
    This contradicts f1 <> f2. *)

Theorem tier_crossing_safety :
  forall r1 r2 f1 f2,
    f1 <> f2 ->
    legally_held r1 f1 ->
    legally_held r2 f2 ->
    ref_addr r1 = ref_addr r2 ->
    (** At most one reference is writable *)
    ~ (is_writable r1 /\ is_writable r2).
Proof.
  intros r1 r2 f1 f2 Hneq Hheld1 Hheld2 Haddr [Hw1 Hw2].
  apply Hneq.
  assert (Ho1 := legally_held_writable_is_owner r1 f1 Hheld1 Hw1).
  assert (Ho2 := legally_held_writable_is_owner r2 f2 Hheld2 Hw2).
  rewrite Haddr in Ho1. rewrite Ho1 in Ho2. exact Ho2.
Qed.

(** ** Theorem 5: Region isolation

    Reference: MEMORY_MODEL.md §7.8.6

    If a fiber owns all addresses in a region, then any other fiber
    holding a reference to a region address can only hold a frozen
    (read-only) reference. The owning fiber has exclusive mutable access.

    This formalizes the invariant from MEMORY_MODEL.md §7.8.6:
    "If ptr points into region r owned by fiber F, then only
    fiber F can dereference ptr [mutably]." *)

Theorem region_isolation :
  forall r fid_owner fid_holder rgn,
    (** All region addresses owned by fid_owner *)
    (forall addr, In addr rgn -> addr_owner addr = fid_owner) ->
    (** Reference points into the region *)
    In (ref_addr r) rgn ->
    (** Reference legally held by a different fiber *)
    legally_held r fid_holder ->
    fid_owner <> fid_holder ->
    (** The reference must be frozen (read-only) *)
    ref_mut r = Mut_Frozen.
Proof.
  intros r fid_owner fid_holder rgn Hown Hin Hheld Hneq.
  destruct (ref_mut r) eqn:Hmut.
  - (* Mut_Mutable: legally_held gives addr_owner = fid_holder,
       but Hown gives addr_owner = fid_owner. Contradiction. *)
    exfalso. apply Hneq.
    unfold legally_held in Hheld. rewrite Hmut in Hheld.
    specialize (Hown (ref_addr r) Hin).
    rewrite Hown in Hheld. exact Hheld.
  - (* Mut_Frozen *) reflexivity.
Qed.

(** Corollary: region isolation implies no writable cross-fiber access *)

Corollary region_isolation_no_write :
  forall r fid_owner fid_holder rgn,
    (forall addr, In addr rgn -> addr_owner addr = fid_owner) ->
    In (ref_addr r) rgn ->
    legally_held r fid_holder ->
    fid_owner <> fid_holder ->
    ~ is_writable r.
Proof.
  intros r fid_owner fid_holder rgn Hown Hin Hheld Hneq Hwrite.
  unfold is_writable in Hwrite.
  assert (Hfrozen := region_isolation r fid_owner fid_holder rgn
                        Hown Hin Hheld Hneq).
  rewrite Hwrite in Hfrozen. discriminate.
Qed.

(** ** Composition: region isolation + generation check

    Combines Theorems 2 and 5: if another fiber holds a reference
    into a region, it must be frozen. If the region is destroyed,
    the generation check detects the stale reference. *)

Corollary region_crossing_detected :
  forall r fid_owner fid_holder rgn M,
    ref_tier r = Tier_Region ->
    (forall addr, In addr rgn -> addr_owner addr = fid_owner) ->
    In (ref_addr r) rgn ->
    legally_held r fid_holder ->
    fid_owner <> fid_holder ->
    NoDup rgn ->
    ref_gen r = current_gen M (ref_addr r) ->
    (** After region destruction, staleness detected *)
    current_gen (region_destroy rgn M) (ref_addr r) <> ref_gen r.
Proof.
  intros r fid_owner fid_holder rgn M Htier Hown Hin Hheld Hneq Hnd Hgen.
  assert (Hfrozen := region_isolation r fid_owner fid_holder rgn
                        Hown Hin Hheld Hneq).
  assert (Hcross : can_cross_fiber r).
  { unfold can_cross_fiber. rewrite Htier. rewrite Hfrozen. exact I. }
  exact (region_checked_cross r rgn M Htier Hfrozen Hcross Hnd Hin Hgen).
Qed.

End FiberSafety.

(** ** Summary

    Phase 10 establishes the following:

    1. stack_no_cross: Stack-tier references cannot cross fiber
       boundaries (stack is fiber-local).
    2. region_checked_cross: When a frozen region reference crosses
       to another fiber and the region is destroyed, the generation
       check detects the stale reference.
    3. persistent_free_cross: Persistent-tier references can cross
       fiber boundaries freely (reference-counted).
    4. tier_crossing_safety: At most one of two references to the
       same address (held by different fibers) can be writable.
       This is data race freedom by construction.
    5. region_isolation: A fiber holding a reference to another
       fiber's region can only hold a frozen (read-only) reference.

    Theorems 4 and 5 are parameterized over an addr_owner function
    (via Section). When the Section closes, addr_owner becomes a
    function parameter.

    Status: 0 Admitted, 0 Axioms.
    Section variables: addr_owner (1).
*)
