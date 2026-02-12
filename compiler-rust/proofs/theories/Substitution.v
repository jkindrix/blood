(** * Blood Core Calculus — Substitution

    This file defines substitution for de Bruijn indexed terms and
    proves the key substitution lemma: substitution preserves typing.

    Reference: FORMAL_SEMANTICS.md §7 (Progress and Preservation)
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
From Stdlib Require Import PeanoNat.
From Stdlib Require Import Lia.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.

(** ** Shifting (de Bruijn)

    Shift free variables in an expression by [d] starting at cutoff [c].
    This is needed to avoid variable capture during substitution. *)

Fixpoint shift_expr (d : nat) (c : nat) (e : expr) : expr :=
  match e with
  | E_Var x =>
      if c <=? x then E_Var (x + d)
      else E_Var x
  | E_Const k => E_Const k
  | E_Lam T body =>
      E_Lam T (shift_expr d (S c) body)
  | E_App e1 e2 =>
      E_App (shift_expr d c e1) (shift_expr d c e2)
  | E_Let e1 e2 =>
      E_Let (shift_expr d c e1) (shift_expr d (S c) e2)
  | E_Annot e1 T =>
      E_Annot (shift_expr d c e1) T
  | E_Record fields =>
      E_Record (map (fun '(l, ei) => (l, shift_expr d c ei)) fields)
  | E_Select e1 l =>
      E_Select (shift_expr d c e1) l
  | E_Extend l e1 e2 =>
      E_Extend l (shift_expr d c e1) (shift_expr d c e2)
  | E_Perform eff op e1 =>
      E_Perform eff op (shift_expr d c e1)
  | E_Handle h e1 =>
      E_Handle (shift_handler d c h) (shift_expr d c e1)
  | E_Resume e1 =>
      E_Resume (shift_expr d c e1)
  end

with shift_handler (d : nat) (c : nat) (h : handler) : handler :=
  match h with
  | Handler hk e_ret clauses =>
      Handler hk
              (shift_expr d (S c) e_ret)  (** return binds one var *)
              (map (shift_op_clause d c) clauses)
  end

with shift_op_clause (d : nat) (c : nat) (cl : op_clause) : op_clause :=
  match cl with
  | OpClause eff op body =>
      OpClause eff op (shift_expr d (S (S c)) body)
      (** binds arg and resume *)
  end.

(** ** Substitution

    [subst j s e] substitutes expression [s] for variable [j] in [e]. *)

Fixpoint subst (j : nat) (s : expr) (e : expr) : expr :=
  match e with
  | E_Var x =>
      if j =? x then s
      else if j <? x then E_Var (x - 1)  (** shift down *)
      else E_Var x
  | E_Const k => E_Const k
  | E_Lam T body =>
      E_Lam T (subst (S j) (shift_expr 1 0 s) body)
  | E_App e1 e2 =>
      E_App (subst j s e1) (subst j s e2)
  | E_Let e1 e2 =>
      E_Let (subst j s e1) (subst (S j) (shift_expr 1 0 s) e2)
  | E_Annot e1 T =>
      E_Annot (subst j s e1) T
  | E_Record fields =>
      E_Record (map (fun '(l, ei) => (l, subst j s ei)) fields)
  | E_Select e1 l =>
      E_Select (subst j s e1) l
  | E_Extend l e1 e2 =>
      E_Extend l (subst j s e1) (subst j s e2)
  | E_Perform eff op e1 =>
      E_Perform eff op (subst j s e1)
  | E_Handle h e1 =>
      E_Handle (subst_handler j s h) (subst j s e1)
  | E_Resume e1 =>
      E_Resume (subst j s e1)
  end

with subst_handler (j : nat) (s : expr) (h : handler) : handler :=
  match h with
  | Handler hk e_ret clauses =>
      Handler hk
              (subst (S j) (shift_expr 1 0 s) e_ret)
              (map (subst_op_clause j s) clauses)
  end

with subst_op_clause (j : nat) (s : expr) (cl : op_clause) : op_clause :=
  match cl with
  | OpClause eff op body =>
      OpClause eff op
               (subst (S (S j)) (shift_expr 2 0 s) body)
  end.

(** ** Notation for substitution *)

Notation "e [ j ':=' s ]" := (subst j s e) (at level 20, left associativity).

(** ** Context removal

    Remove the [j]-th element from a context. *)

Fixpoint remove_nth {A : Type} (j : nat) (l : list A) : list A :=
  match j, l with
  | 0, _ :: rest => rest
  | S n, x :: rest => x :: remove_nth n rest
  | _, [] => []
  end.

(** ** Weakening Lemma

    If Γ ⊢ e : T / ε then (Γ with extra binding) ⊢ (shifted e) : T / ε *)

Lemma weakening :
  forall Sigma Gamma Delta e T eff S n,
    has_type Sigma Gamma Delta e T eff ->
    has_type Sigma
             (firstn n Gamma ++ S :: skipn n Gamma)
             ((Lin_Unrestricted, false) :: Delta)
             (shift_expr 1 n e) T eff.
Proof.
  (* Proof by induction on the typing derivation.
     Each case shifts the expression and adjusts the context.
     The key insight is that shifting preserves the typing structure
     by adjusting de Bruijn indices past the insertion point. *)
Admitted.  (* TODO: complete proof *)

(** ** Substitution Preserves Typing

    This is THE key lemma for type preservation.

    Reference: FORMAL_SEMANTICS.md §12.1

    Lemma substitution_preserves_typing :
      ∀ Γ x e v T S ε,
        Γ, x : S ⊢ e : T / ε →
        Γ ⊢ v : S / pure →
        Γ ⊢ e[v/x] : T / ε.
*)

Theorem substitution_preserves_typing :
  forall Sigma Gamma Delta e v T S eff,
    has_type Sigma (S :: Gamma) ((Lin_Unrestricted, false) :: Delta) e T eff ->
    has_type Sigma Gamma Delta v S Eff_Pure ->
    has_type Sigma Gamma Delta (subst 0 v e) T eff.
Proof.
  intros Sigma Gamma Delta e v T S eff Htype Hval.
  (* Proof by induction on the derivation of has_type for e.

     The proof follows the standard structure for substitution lemmas
     in de Bruijn-indexed calculi:

     - Variable case: If the variable matches j, replace with v and use Hval.
       If the variable is different, adjust the index.

     - Lambda case: The body is under a binder, so we shift v and
       substitute at index j+1. The induction hypothesis applies.

     - Application case: Substitute in both subexpressions.

     - Let case: Similar to lambda for the body.

     - Handle case: Substitute in both the body and handler clauses.

     - Perform case: Substitute in the argument expression.

     Each case uses the induction hypothesis and the weakening lemma
     to handle shifts under binders.
  *)
Admitted.  (* TODO: complete proof by induction on Htype *)

(** ** Multi-substitution

    Substitute multiple values simultaneously. Used for handler clause
    instantiation. *)

Fixpoint multi_subst (vals : list expr) (e : expr) : expr :=
  match vals with
  | [] => e
  | v :: rest =>
      multi_subst rest (subst 0 v e)
  end.

(** ** Substitution commutes with shifting *)

Lemma shift_subst_commute :
  forall e d c j s,
    c <= j ->
    shift_expr d c (subst j s e) =
    subst (j + d) (shift_expr d c s) (shift_expr d c e).
Proof.
  intros e.
  induction e; intros dd cc jj ss Hcj.

  - (* E_Var x *)
    simpl.
    destruct (jj =? v) eqn:Hjv.
    + (* jj = v *)
      apply Nat.eqb_eq in Hjv. subst v.
      simpl.
      destruct (cc <=? jj) eqn:Hccj.
      * simpl.
        destruct (jj + dd =? jj + dd) eqn:Heq.
        { reflexivity. }
        { apply Nat.eqb_neq in Heq. lia. }
      * apply Nat.leb_gt in Hccj. lia.
    + (* jj <> v *)
      destruct (jj <? v) eqn:Hjv'.
      * (* jj < v *)
        apply Nat.ltb_lt in Hjv'.
        simpl.
        destruct (cc <=? v - 1) eqn:Hcv1.
        { apply Nat.leb_le in Hcv1. simpl.
          destruct (cc <=? v) eqn:Hcv.
          { simpl.
            destruct (jj + dd =? v + dd) eqn:Hjdvd.
            { apply Nat.eqb_eq in Hjdvd. apply Nat.eqb_neq in Hjv. lia. }
            destruct (jj + dd <? v + dd) eqn:Hltjv.
            { f_equal. lia. }
            { apply Nat.ltb_ge in Hltjv. lia. }
          }
          { apply Nat.leb_gt in Hcv. lia. }
        }
        { apply Nat.leb_gt in Hcv1. simpl.
          destruct (cc <=? v) eqn:Hcv.
          { apply Nat.leb_le in Hcv. assert (cc = v) by lia. subst cc. simpl.
            destruct (jj + dd =? v + dd) eqn:Hjdvd.
            { apply Nat.eqb_eq in Hjdvd. apply Nat.eqb_neq in Hjv. lia. }
            destruct (jj + dd <? v + dd) eqn:Hltjv.
            { f_equal. lia. }
            { apply Nat.ltb_ge in Hltjv. lia. }
          }
          { apply Nat.leb_gt in Hcv. lia. }
        }
      * (* jj >= v *)
        apply Nat.ltb_ge in Hjv'. apply Nat.eqb_neq in Hjv.
        assert (jj > v) by lia. simpl.
        destruct (cc <=? v) eqn:Hcv.
        { simpl.
          destruct (jj + dd =? v + dd) eqn:Hjdvd.
          { apply Nat.eqb_eq in Hjdvd. lia. }
          destruct (jj + dd <? v + dd) eqn:Hltjv.
          { apply Nat.ltb_lt in Hltjv. lia. }
          { reflexivity. }
        }
        { simpl.
          destruct (jj + dd =? v) eqn:Hjdv.
          { apply Nat.eqb_eq in Hjdv. lia. }
          destruct (jj + dd <? v) eqn:Hltjdv.
          { apply Nat.ltb_lt in Hltjdv. lia. }
          { reflexivity. }
        }

  - (* E_Const *) reflexivity.

  - (* E_Lam: needs shift commutation lemma *)
    simpl. f_equal.
    rewrite IHe with (c := S cc) (j := S jj) by lia.
    (* Remaining: show shift commutation for ss *)
    admit.

  - (* E_App *)
    simpl. f_equal; [apply IHe1 | apply IHe2]; assumption.

  - (* E_Let: similar to E_Lam *)
    simpl. f_equal.
    + apply IHe1. assumption.
    + rewrite IHe2 with (c := S cc) (j := S jj) by lia. admit.

  - (* E_Annot *)
    simpl. f_equal. apply IHe. assumption.

  - (* E_Record: nested list *)
    simpl. f_equal.
    induction l as [| [lbl ei] rest IHl]; [reflexivity | simpl; admit].

  - (* E_Select *)
    simpl. f_equal. apply IHe. assumption.

  - (* E_Extend *)
    simpl. f_equal; [apply IHe1 | apply IHe2]; assumption.

  - (* E_Perform *)
    simpl. f_equal. apply IHe. assumption.

  - (* E_Handle: nested structure *)
    simpl. f_equal.
    + destruct h as [hk e_ret clauses]. simpl. f_equal; admit.
    + apply IHe. assumption.

  - (* E_Resume *)
    simpl. f_equal. apply IHe. assumption.
Admitted.  (* Partial: var case done, needs shift commutation and nested list lemmas *)

(** ** Generalized lemma: shifting then substituting cancels *)

Lemma shift_then_subst_general :
  forall e s cutoff,
    subst cutoff s (shift_expr 1 cutoff e) = e.
Proof.
  intros e.
  induction e; intros s cutoff.

  - (* E_Var x *)
    simpl.
    destruct (cutoff <=? v) eqn:Hle.
    + (* cutoff <= v *)
      simpl.
      destruct (cutoff =? v + 1) eqn:Heq.
      * (* cutoff = v + 1, but cutoff <= v, contradiction *)
        apply Nat.eqb_eq in Heq.
        apply Nat.leb_le in Hle.
        lia.
      * destruct (cutoff <? v + 1) eqn:Hlt.
        { (* cutoff < v + 1, so we return v + 1 - 1 = v *)
          f_equal.
          apply Nat.leb_le in Hle.
          lia.
        }
        { (* cutoff >= v + 1, but cutoff <= v, contradiction *)
          apply Nat.ltb_ge in Hlt.
          apply Nat.leb_le in Hle.
          lia.
        }
    + (* cutoff > v *)
      simpl.
      destruct (cutoff =? v) eqn:Heq.
      * (* cutoff = v, but cutoff > v, contradiction *)
        apply Nat.eqb_eq in Heq.
        apply Nat.leb_gt in Hle.
        lia.
      * destruct (cutoff <? v) eqn:Hlt.
        { (* cutoff < v, but cutoff > v, contradiction *)
          apply Nat.ltb_lt in Hlt.
          apply Nat.leb_gt in Hle.
          lia.
        }
        { reflexivity. }

  - (* E_Const *) reflexivity.
  - (* E_Lam *) simpl. f_equal. apply IHe.
  - (* E_App *) simpl. f_equal; [apply IHe1 | apply IHe2].
  - (* E_Let *) simpl. f_equal; [apply IHe1 | apply IHe2].
  - (* E_Annot *) simpl. f_equal. apply IHe.

  - (* E_Record: nested list requires custom induction, admit for now *)
    simpl. f_equal.
    induction l as [| [lbl ei] rest IHl].
    + reflexivity.
    + simpl. admit.

  - (* E_Select *) simpl. f_equal. apply IHe.
  - (* E_Extend *) simpl. f_equal; [apply IHe1 | apply IHe2].
  - (* E_Perform *) simpl. f_equal. apply IHe.

  - (* E_Handle: handler clauses require custom induction *)
    simpl. f_equal.
    + destruct h as [hk e_ret clauses].
      simpl. f_equal; admit.
    + apply IHe.

  - (* E_Resume *) simpl. f_equal. apply IHe.
Admitted.  (* Partial: only record fields and handler clauses need custom mutual induction *)

(** ** Identity substitution *)

Lemma subst_shift_cancel :
  forall e v,
    subst 0 v (shift_expr 1 0 e) = e.
Proof.
  intros e v.
  apply shift_then_subst_general.
Qed.
